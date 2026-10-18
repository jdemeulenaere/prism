# Design: instance-cache

## Context

See `proposal.md` for motivation. This is item 21 of "Then: the road to Milestone 2" in `ROADMAP.md`. It was split out of `monomorphization` by that change's D1, and `monomorphization`'s D13 recorded its direction. It also takes up the follow-ups of `abi-cache`: what an instance key must cover (its D9 and D12), the target triple in object-code keys (its D5), and records written before the run is known to pass. It takes up `codegen-core`'s "cache generated code per file" too (its D20 and D24). The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

**Baseline.** This change was proposed while `monomorphization` was being implemented (its task 1.1). It lands after `monomorphization` is archived, and its deltas are written against the requirements as that change leaves them. In `generics` it renames and modifies "Polymorphic Recursion Across Files Is Rejected When Compiled", which `monomorphization` adds, which `monomorphization` added. In `code-generation` it modifies "Tier 1 Compiles Elaborated MIR in Memory" as `monomorphization` leaves it. The functions of `monomorphization` named below (`GenericRecord`, `instantiate`, `collect_instances`, `InstanceUnit`, `instance_symbol`, `Linker::add_instance`, `check_instances`) are those of its `tasks.md`. Before starting, compare each with the landed code, and compare every MODIFIED block with the archived main text.

**Reconciliation (2026-10-18, before implementing).** `monomorphization`, `closures`, `escaping-closures` and `collections` are archived, and the compile-speed work (sparse borrow-check states, linear equality glue, the `compile_timing` shapes) has landed. Every MODIFIED block was compared with the main text again, and the functions this design and `tasks.md` name with the landed code:

- `code-generation` "Tier 1 Compiles Elaborated MIR in Memory" now compiles closure bodies (`closures`) and has the scenario "A closure is compiled". The delta had dropped both; it keeps them, and adds the cache clause and "A warm run starts no compilation".
- `compilation-and-abi` "Escaping Parameters in the ABI" (`escaping-closures`) says that the ABI and interface hashes cover the flags. Since the interface hash is removed, it is MODIFIED to say the ABI hash, with its scenario "Marking a parameter escaping changes the hashes".
- `generics` "Polymorphic Recursion Across Files Is Rejected When Compiled" is now in the main spec, so the RENAMED and MODIFIED blocks apply cleanly. The other MODIFIED blocks (`build-cache`, `execution` "Program Arguments and Options", the rest of `compilation-and-abi`) match the main text apart from their intended edits.
- Landed names: `GenericRecord`, `GenericRecord::of_file`, `GenericRecord::instantiate` (which instantiates a body with the closures lowered inside it, D11 of `closures`), `collect_instances`, `InstanceUnit`, `instance_symbol`, `callee_symbol`, `check_instances`, `Linker::add_file`, `Linker::add_instance`, `dump_instances` and `find_main` are as named. `growing_cycle` lives in `prism_typed_tree::instantiation` beside `growing_cycles` (every cycle, not the first), which `instances.rs::check_cross_file_recursion` uses to report each cycle once; planning reuses the latter and that function.
- The MIR the codec must cover grew: closures (`Rvalue::Closure`, `Capture`, `CaptureMode`, `Body::capture_modes`, `ownership_reason`, `LocalDeclaration::escaping`, `TerminatorKind::Call::escaping`), lists (`Type::List`, `Projection::Index`, the list built-ins) and templates (`Rvalue::Template`). D4's exhaustive matches cover them.
- Code units now hold closure descriptors, data objects with relocations to slots and to environment drop functions (`unit.rs::descriptor`), so D8's data relocations are needed from the start, not later.

What exists, or will once `monomorphization` lands:

- **Hashes** (`crates/prism-abi/src/signature.rs`). An exported generic function records `generic_body_hash`, a span-free hash of its body's AST. `FileAbi::calculate_hashes` computes the interface hash (the ABI walk without body hashes) and the ABI hash (the interface hash and every body hash). `prism abi` prints both.
- **The cache** (`crates/prism-cli/src/cache.rs`, format 1). It holds text entries `v1/<2 hex>/<key>` with a header, body lines and a checksum. A check entry holds a file's `InstanceRequest` lines: a direct import's generic, a hash of its type arguments and its display. An instance record is an empty entry keyed by `instance_key` (the generic's identity, signature hash, body hash and type-arguments hash). The check key covers the format, the compiler identity (version and executable hash), the module path, the source and the direct imports' interface hashes.
- **`prism check`** (`workspace.rs`, `validate_and_extract_abi`). It extracts every ABI (Phase 1, `extract_abis`, which also builds every file's type definitions, private ones included). Then, per file in topological order, it reuses the verdict or runs Phase 2 (`elaborate_file`), and collects `instance_requests` from the elaborated MIR (direct imports only, no transitive instance). `instance_outcomes` looks up the instance records.
- **`prism run`** (`run.rs`, `compile_program`). It ignores the cache. It elaborates every file, runs `find_main` and `check_file`, then `monomorphization`'s `collect_instances` (records built in memory, `E6004`, the worklist with `instantiate`), `check_instances`, and links every file unit and instance unit into one `JITModule` through `Linker::add_file` and `Linker::add_instance`.
- **The linker** (`crates/prism-codegen/src/link.rs`). It compiles each function into the module with `Module::define_function`. Large functions go through `context.compile` with the single-pass ISA, `ModuleReloc::from_mach_reloc` and `Module::define_function_bytes`. So defining a function from bytes and relocations already works in the Tier 1 JIT.
- **The ISA** (`isa.rs`). The host triple and CPU features come from `cranelift_native`. Fixed settings (`opt_level=none`, the verifier in debug builds, frame pointers, inline probestack) apply, and large functions use a second ISA with the single-pass allocator.

## Goals / Non-Goals

**Goals:**
- An instance's code is generated again exactly when something its code depends on changes. That covers its body, the private types and callee signatures its body uses, and its type arguments' definitions. A file's code is generated again exactly when its check key changes.
- A warm `prism run` of an unchanged program runs no Phase 2, no MIR lowering, no instantiation and no Cranelift compilation. A body edit compiles one file unit, plus the instances of the edited generics.
- `prism check` sees every instance of the program, and cross-file polymorphic recursion, from small per-file summaries, without reading MIR.
- `hot-reload` can compute, cheaply and without MIR, which units and instances changed after an edit.
- Determinism and the zero-marker invariant are kept. Entries are functions of their keys, keys hold no path, and the cache stays per user.

**Non-Goals:**
- Skipping parsing and Phase 1 on a warm run. The resolver still parses every file (`abi-cache` D4). Its follow-up, cached `.abi` and import-list entries, stays on the roadmap.
- Positions relative to the declaration in records (a follow-up of `monomorphization` D13), per-symbol fingerprints, eviction, sharing equal-layout instances.
- Cached code in dumps (`dump-clif`, `dump-mir`, `--instances`), which always compile afresh.
- Tier 2 object files. The artifacts here are Tier 1's, and Tier 2 will have its own entries.

## Decisions

### D1. This change also caches generated code (autonomous)

The roadmap item names records, hashes and keys. The alternative is a later `code-cache` change for the machine code of file units and instance units. Code caching is folded in here for three reasons. First, an instance key that guards no code is only bookkeeping: `abi-cache` shipped such records, and their known gaps went untested for two changes. Second, the decisions are shared. Code keys are check keys plus a target identity (D8), instance code keys are instance keys (D6), and the entry format must carry binary bodies (D11) for records anyway. Third, compile speed is a primary criterion. A warm `prism run` that still type-checks, lowers and compiles every file wastes exactly the work this change makes avoidable, and `hot-reload`'s startup is a `prism run`. The tasks are grouped so that hashes and records (groups 1 to 4) can land before code entries (groups 5 to 6).

Rejected: a separate `code-cache` change, which would leave instance keys unverified by any consumer. Caching only records and verdicts, so that `prism run` would skip Phase 2 but still compile everything, saves the cheaper half: Cranelift compilation dominates Tier 1 time on large files (`compile_timing`'s `clif` column).

### D2. The Monomorphization Hash is the hash of the record's canonical encoding (autonomous, from `monomorphization` D13)

For every generic function and extension of a file that passes Phase 2, private ones included, the Monomorphization Hash is `BLAKE3(encode(record))`. Here `record` is `monomorphization`'s `GenericRecord`: the elaborated polymorphic MIR of its body, default generators and closures, and its restricted type table. `encode` is the canonical codec of D4. Hashing the encoding means that the hash covers exactly what is stored and decoded, so no second hashing walk can drift from the codec.

Like D13, the hash covers source positions (line and column of every span), because an instance reports runtime errors at them. The record's spans are encoded relative to nothing, so an edit before a generic regenerates its instances, and only them (no importer, no file unit other than the defining file's). Positions relative to the declaration's start would avoid that. But the instance's error data would then need the declaration's line from somewhere outside the instance, and that couples the instance to its file's unit. It stays the follow-up D13 named. The hash covers:

- the private types the body holds, through the record's restricted table;
- every callee's signature, through each call's recorded parameter types, destination type and type arguments (a helper's parameter becoming `&Int` adds a borrow in the MIR);
- the generic's own signature, through its locals.

A span is encoded whole, with its byte offsets, which diagnostics use to underline the source, as well as its line and column. So any edit before a generic that moves its source changes its hash, even one that keeps the line count; only edits after it, or that keep every byte offset, do not. Encoding only lines and columns would let one record entry stand for records with different offsets, and an instance's `E6001` would then be underlined at stale offsets (implementation note, 2026-10-18: the scenarios of a helper's body edit declare the helper after the generic for this reason).

It covers no other body. A non-generic helper is called by symbol through its slot (`monomorphization` D2), so a helper's body edit changes only its file's unit, and the instance's cached code calls the new helper.

Type-parameter names are encoded with their indices and bounds, because a decoded record must equal the original and dumps print the names. So renaming a type parameter regenerates the generic's instances, though it re-checks no importer, since the ABI hash ignores the names (`generics`). Rejected: keeping the Phase 1 AST hash (it misses the private-type and callee-signature cases, as `monomorphization` D13 showed); hashing the defining file (every edit would regenerate every instance of the file); excluding positions (an instance's runtime error would report a stale line).

### D3. One ABI hash (following `monomorphization` D13)

`ExportedSymbol::Function` loses `generic_body_hash`, and `FileAbi` loses `interface_hash`. `calculate_hashes` becomes `calculate_hash`, the former interface walk, whose result is `abi_hash`. `ExportedSymbol::interface_hash`, which keyed `abi-cache`'s instance records, is removed with them, and `generic_function_of` and `generic_body_hash_of` go. The deterministic hermetic builds invariant already says "direct import ABI hashes", and the check key becomes exactly that. `prism abi` prints `ABI Hash:` only. The `prism check` report's `ABI: 0x...` now prints the value the interface hash had.

The name "ABI hash" is kept rather than "interface hash", because the invariant, the report and `prism abi` use it. The cache format version moves to 2 (D11), so no format 1 entry is ever read against the new keys. Rejected: keeping two names for one value.

### D4. A canonical, hand-written binary codec for records (autonomous)

`crates/prism-mir/src/codec.rs` adds `Encoder` (appending to a `Vec<u8>`) and `Decoder` (reading a `&[u8]`, returning `None` on any malformed input). Each MIR and typed-tree type the records hold gets an exhaustive `encode` and `decode`: `Body` and its arenas, places, operands, rvalues, statements, terminators, `Type`, `TypeParameter`, `DeclarationPath`, `Span`, and the definitions and markers of a `TypeTable`. The format is fixed-width little-endian integers, LEB128 lengths, UTF-8 strings with their lengths, and a tag byte per enum variant. A `TypeTable` encodes its definitions in `NominalId` order, since it is a `BTreeMap`. Derived indices, such as member indices, are rebuilt on decode, never encoded. No `HashMap` is iterated, so the bytes are a function of the record alone.

The matches are exhaustive, with no wildcard arm. A MIR construct added later, by `compose-transform` for instance, is then a compile error in the codec until it is encoded. A test round-trips the elaborated MIR of every fixture of the corpus (encode, decode, compare with `PartialEq`, validate), which also catches a field left out.

Rejected: `serde` with `bincode` or `postcard`. That would be the first serialization dependency, derives on every typed-tree and MIR type, and determinism at the mercy of field types (a `HashMap` would encode in iteration order and silently break the hash). Also rejected: the text dump as the stored form, which has no parser and is several times larger.

### D5. Summaries in check entries, records in entries of their own (autonomous)

The roadmap says "generic records carried in the defining file's check entries". This design splits what a warm check needs from what only compiling a missed instance needs:

- **The check entry holds the file's summary** (`prism_mir::FileSummary`, new `summary.rs`):
  - `entry_point`: whether the file declares a `main` that `find_main` accepts (D9);
  - `roots`: the instance requests of its compiled, non-generic bodies, meaning every call of a `Callee::Function` or `Callee::Default` with concrete type arguments, any module's generics included (unlike `abi-cache`'s direct-import-only requests);
  - `generics`: per generic function or extension, its path, its type-parameter count, its Monomorphization Hash, and its **generic calls** (callee, kind, type arguments in terms of its own parameters, span), taken from its body, default generators and closures as `monomorphization` D8's graph counts them.

  A summary is small (a few dozen bytes per generic call) and is everything `prism check` and planning (D7) read.
- **Each record is an entry of kind `record`**, keyed by the format, the compiler identity and the Monomorphization Hash: content-addressed, shared by every file and checkout that produces the same record. It is read only when an instance's code misses (D9). If it is missing or corrupt, the generic's file is elaborated again and its records rewritten, so a record entry is an optimization, never a correctness input.

Rejected: whole records inside the check entry, which a warm `prism check` would read and decode for every generic of the program though it compiles nothing. Also rejected: not storing records and elaborating the defining file again on every instance miss. That is correct and is the fallback, but it makes "app starts calling `keep<String>`" re-check `lib/box` on every such edit, and hot reload hits it on every new instance.

### D6. What an instance key covers (autonomous, answering `abi-cache` D9)

An instance's code key is the BLAKE3 hash of: the entry kind `instance`, the format version, the compiler identity, the target identity (D8), the instance's symbol (`monomorphization` D4, which names the generic and the module-qualified type arguments), its generic's Monomorphization Hash (D2), and the **type-argument definitions hash**.

The type-argument definitions hash covers the definitions, markers and marker parameters included, of every struct and enum the type arguments reach. The closure runs through fields and payloads, over every module, and the definitions are sorted by module and name. Definitions come from Phase 1a (`type_definitions`, which `extract_abis` already computes for every file of the program on every run) and are hashed with `AbiTypeDefinition::hash_into`. This is the same structural hash the ABI uses, so a field, a field type, a marker or a variant change is seen. Built-in types hash by structure. `extract_abis` returns these definitions beside the ABIs (`ProgramDefinitions`). Each nominal's closure hash is memoized per run, so the hashes cost time linear in the program's distinct definitions.

What it does not need:

- the generic's signature, which the record holds (its argument and return locals), so `drop(x: T)` becoming `drop(x: &T)` changes the hash;
- private helpers' bodies, which instances call by symbol (D2);
- other generics' records, which are other instances with their own keys;
- the requester's identity, so one instance serves every caller (`monomorphization` D3, D5).

The private helpers a body calls are covered through their signatures in the record. `abi-cache` asked whether their bodies must be keyed, and the answer is no.

### D7. Instances are planned per program from summaries, in `prism check` too (autonomous)

`monomorphization`'s `collect_instances` is split into two steps.

1. **`plan_instances(summaries) -> Result<Vec<PlannedInstance>, Vec<CodegenError>>`** (`crates/prism-codegen/src/instances.rs`). It first runs `growing_cycles` over every generic call of every summary and reports a cycle as `E6004`. Then it runs the worklist of `monomorphization` D5 over summaries: the roots are every file's `roots`, and an instance's requests are its generic's calls with its type arguments substituted (`Type::substitute`). Requests are deduplicated by symbol before anything else is computed. A `PlannedInstance` holds the symbol, the generic's path and body kind (function or `#default(<parameter>)`), the concrete type arguments and the Monomorphization Hash. The output is sorted by symbol.
2. **`build_instance_unit(record, planned, types) -> InstanceUnit`**, which is `monomorphization`'s instantiation of one request (D6 of that change). It runs only for instances whose code misses.

Both `prism check` and `prism run` plan, from summaries alone. A reused file's summary comes from its check entry, a checked file's from its fresh MIR. So `prism check` lists every instance of the program, transitively and through modules a caller does not import, and finds `E6004` across an import cycle even when every file's verdict is reused. A file with a cycle passes Phase 2 alone, so its entry exists. `collect_instances` stays as `plan_instances` followed by `build_instance_unit` for every instance, for the dumps and `Program::compile`.

The substituted requests are the same set `monomorphization`'s worklist finds by instantiating bodies. A test runs both over the fixture corpus and compares the symbol sets.

**`E6004` survives** (`monomorphization`'s open question). It is the same mistake as `E4027`, but it is found by a whole-program step over several files' summaries rather than by one file's type checker, and its code says so. The catalog row changes from "found when compiled" to "found by `prism check` and `prism run`". Rejected: retiring it for a cross-file `E4027`, which would put a program-level check into a per-file stage's code range and blur which stage reports what.

### D8. Code entries: relocatable artifacts, one definition path, keyed by the target identity (autonomous)

A compiled unit becomes a **`UnitArtifact`** (new `crates/prism-codegen/src/artifact.rs`) holding:

- **functions**: each with its symbol, linkage, whether it has a slot, its Cranelift signature, alignment, code bytes and relocations;
- **data objects**: each with its name, whether it is writable, alignment, contents or zero size, and data relocations, which closure descriptors hold;
- **relocations**: an offset, a `Reloc` kind, an addend and a **symbolic target**, meaning a function or data object by name (another function of the unit, a glue function, a `<symbol>#slot`, a runtime function, `STACK_LIMIT_SYMBOL`) or a `LibCall` by name.

`Linker::compile_unit(&MirFile) -> UnitArtifact` lowers the unit as today, compiles every function with `context.compile` (the large-function ISA when `is_large`), and maps every `ModuleReloc` target from the module's `FuncId`/`DataId` to its declared name. `Linker::define_artifact(&UnitArtifact)` declares every name with its linkage and signature, maps the targets back to ids, and calls `define_function_bytes` and `define_data`. `add_file` and `add_instance` are `compile_unit` followed by `define_artifact`. So fresh and cached code take one path. In debug builds, `add_file` and `add_instance` encode and decode every artifact they define, so the whole test suite exercises the codec. A relocation target or `LibCall` the codec cannot name makes that unit uncacheable: it is compiled every run and never written, as `abi-cache` skips unwritable fields.

**Keys.** A file unit's code key is `H("unit", format, compiler, target identity, check key)`. Its check key already covers the module path, the source and the direct imports' ABI hashes, which is everything a file's code depends on (`compilation-and-abi`, Phase 3). An instance's key is D6's.

**The target identity** (`isa::target_identity() -> String`) is the target triple, the `Display` of the shared flags and of the ISA-specific flags of both ISAs (the normal one and the large-function one), the CPU features `cranelift_native` detected included. A cache shared over a network home directory therefore never runs code built for another CPU's features. This closes `abi-cache` D5's follow-up ("the target triple joins keys with the first object-code entry").

Rejected: caching Cranelift IR (`Function`) and compiling it on load, which saves only lowering. Also rejected: object files through `cranelift-object` loaded by a custom loader, a second code path that Tier 2 will need in a different shape anyway. Unwind information is not cached, because Tier 1 registers none for functions defined from bytes (as for large functions today). Prism code never unwinds, since a runtime error ends the process.

### D9. `prism run` uses the cache by default (autonomous)

`run.rs::compile_program_with(entry, &RunOptions)` replaces `compile_program`. `RunOptions` gains `cache: Option<BuildCache>` (`None` in the library default, so the fixture harness and library tests stay hermetic, as `abi-cache` D10 did for `check_path`) and `explain_cache`. The `prism` binary opens the cache from the environment unless `--no-cache`. Per file, in topological order:

1. Compute the check key and code key. Read the check entry (verdict and summary) and the code entry.
2. If both hit, the file is `reused`: no Phase 2, no lowering, its artifact is defined as read.
3. If only the check entry hits, the file is `compiled`: elaborate it (Phase 2 runs to get MIR, but its verdict is known) and compile it.
4. Otherwise it is `checked`: elaborate it, write its check entry and record entries, `check_file` it (`E6001`), compile it, and write its code entry.

The entry file's `main` comes from its summary's `entry_point` when its check entry hits. Otherwise, and whenever the summary records no valid `main`, the entry file is elaborated and `find_main` reports `E6002`/`E6003` from source as today, since errors are never cached.

Then `plan_instances` runs over every summary. For each planned instance whose code entry hits, its artifact is defined. For each miss, its record is taken from a file elaborated in this run, or from its record entry, or failing both by elaborating its generic's file again (listed as `checked`). Then it is instantiated, `check_instances` runs on the batch of missed instances, it is compiled, and its code entry is written. Errors are reported exactly as without a cache, since a cached unit or instance is one that passed every check.

`--explain-cache` prints to stderr, before `main` runs, because stdout is the program's. Rejected: opt-in caching for `prism run` (`--cache`), which would leave the common path slow. Also rejected: a separate `prism build` that fills the cache, because Prism's development loop is `prism run`.

### D10. `--explain-cache` lists instances once per program (autonomous, BREAKING)

File lines stay in check order: `checked`, `reused` and, under `prism run` only, `compiled` (D9). Instance lines follow, once per program, in symbol order: `instantiate <instance>` when the cache holds no code for the instance's key, and `reused instance <instance>` when it does. Under `prism run` that is what it just did. Under `prism check`, which generates no code, it is what the next run will do. So a check after an edit predicts which instances that edit invalidated, which is the use `abi-cache` built the lines for. `prism check` looks up code entries only when `--explain-cache` asks, which costs one `stat` per instance, and it writes no instance entry. The empty instance records of format 1 are gone.

The `for <path>` suffix is dropped, because an instance is now per program and has no requester (`monomorphization` D3). The display stays `abi-cache` D12's (`lib/box::Box.size<Int>`), not the symbol (`lib/box::lib/box.Box.get<Int>`), so the scenarios of "Instances Shown by Their Identity" keep holding.

Rejected: keeping the per-requester lines (the same instance would be listed under every caller, and transitive ones under no file); printing instance lines only in `prism run` (`prism check` would lose the prediction and its scenario).

### D11. Cache format version 2 with binary bodies (autonomous)

An entry is a text header (`prism-cache 2`, `kind <kind>`, `key <hex>`, `length <bytes>`), the body bytes, then `\nchecksum <hex>\n` over everything before it. The kinds are `check` (verdict and summary), `record`, `unit` and `instance`. Headers stay readable with `head`, and bodies are D4's encoding (summaries, records) or D8's (artifacts). Format 1 entries live under `v1/` and are never read. The layout `v2/<2 hex>/<key>`, atomic writes, `CACHEDIR.TAG` and every-problem-is-a-miss are unchanged. A decode failure is a miss like a bad checksum. Rejected: keeping text bodies, which would need a text codec for MIR and machine code; a separate store per kind, which only spreads one design over several directories.

### D12. The build plan, for `hot-reload`

`plan.rs` (new, `crates/prism-cli/src/plan.rs`) produces a **`BuildPlan`**: for each file its path, module, check key and code key, and for each planned instance its symbol, generic, type arguments and code key. `prism check --explain-cache`, `prism run` and, later, `hot-reload` all build it the same way. `hot-reload` keeps the previous plan in memory. After an edit it re-runs Phase 1, replans, and compiles or loads only the units and instances whose code keys are new, then repoints their slots. `BuildPlan::changed_since(&previous)` returns those. That is "which code units and instances changed", answered without MIR. This change only builds the plan and the comparison, with a unit test. Watching files and repointing slots are `hot-reload`'s.

### D13. A program-wide type table for building instance units

`monomorphization` D5 builds an instance unit's table from the record's table and the requester's restricted table. With cached requesters there may be no requester MIR in memory. So instance units take their type-argument definitions from the program's Phase 1a definitions instead, converted once per run by the type checker's conversion from ABI definitions (`ingest_file_abi`'s loop, extracted as `prism_typechecker::type_definition_from_abi`) into a `TypeTable`, and restricted with `TypeTable::restricted_to`. Every `NominalId` has one definition per program, so this agrees with every requester's table (a debug assertion compares them when both exist). It is built only when an instance misses.

### D14. Entries are written as soon as their unit passes

`abi-cache` noted that instance records were written as soon as a file passed, even when a later file failed, "which must change once a record stands for real code". It need not change. Every entry is a function of its key: a check entry says that this source passes against these ABIs, a record entry that this record has this hash, a code entry that this unit or instance compiles to these bytes. None of those facts depends on whether another file of the run fails. So each entry is written when its own unit passes, and a run that later fails leaves only true entries. Code entries are written only after `check_file` or `check_instances` accepts the unit, so a cached artifact never hides an `E6001`.

### D15. No new capability (autonomous)

The cache's stores, keys and explanation extend `build-cache`. The hashes, records, planning and keys extend `compilation-and-abi`. Artifacts extend `code-generation`, `E6004` in `prism check` extends `generics`, and the run options extend `execution`. Rejected: an `instance-cache` capability, which would split the cache's rules across two specs.

## Risks / Trade-offs

- [A key misses an input, and a stale artifact runs] → keys are derived from the Phase 3 dependency rule that `monomorphization` made explicit (file code depends on its file and direct imports' ABIs, and an instance on its record and type-argument definitions), and the debug-build encode/decode round-trip runs on every definition. The fixture harness runs every executable fixture cold, then warm on the cache the cold run filled, and the warm run must match the snapshots and write no entry. `cache_tests.rs` tests each covered input with an edit (private type, callee signature, type argument field, helper body).
- [Reformatting a library regenerates all its instances] → only instances, never importers' units. Instance compiles are small. Relative positions are the recorded follow-up.
- [Machine code read from a per-user directory is executed] → the same trust model as Go's build cache or `ccache`: whoever can write the user's cache can already run code as that user. The checksum detects corruption, not tampering.
- [Disk use grows faster, since code entries are larger than verdicts] → eviction stays the roadmap follow-up (`abi-cache`'s). Entries are content-addressed, so editing back and forth reuses them.
- [The codec must follow every MIR change] → exhaustive matches make a missing case a compile error, and the corpus round-trip test catches a missing field.
- [Cold runs pay for encoding and writing entries] → measured by the warm-run timing shapes (group 7). Writes are sequential small files. If they show up in `prism run`'s time to `main`, they can move to a thread joined at exit (a lost entry is a miss).
- [Modifying requirements of an unarchived change] → `monomorphization` lands first (Baseline).
- **Compile-time cost.** Everything added is linear, or saves work.
  - Encoding and hashing a record is one pass over its MIR, once per elaborated generic. The cost is proportional to the file's generic bodies, paid only when the file is checked.
  - A summary is one pass over a file's calls.
  - Planning (D7) is Tarjan's components over the generic calls of every summary, linear in them, plus a worklist that substitutes each distinct instance's calls once. The worklist is linear in the program's distinct instances and their generic calls, with a hash lookup per request.
  - Type-argument definitions hashes are memoized per nominal, so they are linear in the program's definitions.
  - Defining a cached artifact is one pass over its bytes and relocations. No Cranelift pass runs, and no Phase 2, lowering or instantiation runs for a cached unit.
  - A warm run of an unchanged program costs Phase 1 (parsing and ABI extraction of every file, already linear) plus reading and checksumming its entries, linear in their size.
  - Under `--explain-cache`, `prism check` adds one entry lookup per instance.

  No analysis is per-instance re-checking. Instance units are still never borrow-checked (`generics` D7). The cost no cache removes is Phase 1 parsing, whose follow-up is listed. The Kotlin-likeness trade-off is nil: the change adds no surface syntax, only two CLI flags on `prism run`.

## Migration Plan

No external users. Format 1 entries are ignored (a new `v2/` directory), so the first run after upgrading is cold. `prism abi` loses its `Interface Hash:` line, and every test or snapshot printing it changes. `--explain-cache` output changes shape (D10), so `cache_tests.rs` is updated with it. Rollback is a revert. Old `v2/` entries would then be ignored by a format 1 compiler.

## Open Questions

- Whether `prism run`'s cache writes should move to a background thread, decided by the warm-run benchmark (group 7).
- The exact text of the `E6004` message reported by `prism check` is `monomorphization`'s, unchanged unless review finds it unclear without a run.
