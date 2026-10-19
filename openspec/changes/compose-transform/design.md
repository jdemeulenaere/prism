# Design: compose-transform

## Context

See `proposal.md` for motivation. This is item 28 of "Then: the road to Milestone 2" in `ROADMAP.md`, the compiler half of the composition layer of `docs/architecture.md` §5. It implements `compose-architecture` D3 (a MIR pass before instantiation), D4 (an explicit composer parameter and change words), D5 (keys from declaration paths), D8 (`remember` keeps `Copy` values), D9 and D10 (stability, skipping and restart scopes) and the language half of D11 (an owned `Composition`), on the runtime protocol of `compose-runtime`, whose D1 leaves group keys, slot type records, restart functions, `remember`, `key` and the owned `Composition` to this change. Each decision is repeated below where this change refines it. The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19), so each decision states its compile-time cost next to its Kotlin-likeness, and its run-time cost where it has one.

**Baseline.** This change was proposed while `escaping-closures` was being implemented and `collections`, `hot-reload`, `instance-cache`, `standard-library` and `compose-runtime` were proposed but not implemented. It needs `compose-runtime` archived, which brings the rest:

- `compose-runtime`: the `prism-compose` crate, `COMPOSE_FUNCTIONS`, `unit.rs::entry_point(symbol)` in the backend, the entry points `prism_compose_start`, `set_content`, `recompose`, `dispose`, `dump`, `start_group`, `start_movable_group`, `start_restart_group`, `end_group`, `end_restart_group`, `skipping`, `skip_to_group_end`, `current_scope`, `invalidate`, `slot`, `set_slot` and `changed`, the `SlotType` record (`#[repr(C)]`: fingerprint, size, alignment, drop, equality, name), the composer as the identifier of the active pass (its D3), restart functions as owned `(Int) -> Unit` values (its D8), the `compose` capability and `/prism/composer`.
- `standard-library`: `crates/prism-library` with `MODULES` and `PRELUDE`, the intrinsic catalog (`crates/prism-typechecker/src/intrinsics.rs`, `E4029`) with signature-typed and rule-typed intrinsics, `Callee::Builtin` calls of intrinsics, members from intrinsic extensions (`E4010`), `E3002` for library-only forms in the workspace.
- `escaping-closures`: `escaping` parameters, owned captures (`CaptureMode::Owned`), heap environments freed by `#drop-environment`, inline one-word environments with a null drop entry.
- `hot-reload`: `contract.rs::TypeFingerprints` (256-bit structural type fingerprints, memoized per load) and slot contracts from each body's declared parameter types.
- `instance-cache`: generic records carried in check entries, encoded by an exhaustive MIR codec, and instances built from records.

The MODIFIED blocks were written against the texts those packets leave: `code-generation` "Each Closure Has a Descriptor" from `escaping-closures`, and `standard-library` "The Compiler Ships the Library" (from `compose-runtime`), "Library Modules Compile Like Workspace Files" and "Intrinsic Calls Are Typed by Their Signatures" (from `standard-library`), so `openspec validate --strict` reports the `standard-library` capability as not yet in the main specs. Compare each with the archived text at apply time.

What exists today:

- **Typed tree and type checker.** `compose fun` and composable function types keep their compose flag (`Type::Fn { is_compose }`), a closure checked against a composable function type is a composable context, and `E4008` rejects composable calls elsewhere, through the compose-context stack of `checker.rs` (`push_compose_context`). A parameter's default value is checked inside the function's compose context (`check_function_body_in_scope`), so a composable call in a composable's default is accepted today.
- **MIR** (`crates/prism-mir`). `Body::is_compose` marks composable functions and closures, and `TerminatorKind::Call { is_compose }` marks composable calls (`build/call.rs`). Default generators are never composable bodies (`build/mod.rs`). Scope exits emit drops and storage ends on every path that leaves a scope (`build/scope.rs`). Closures are separate bodies named by their parent and index (`BodyOwner::Closure`). Records (`record.rs`) hold a generic's elaborated bodies, and `instantiate.rs` substitutes them.
- **Backend** (`crates/prism-codegen`). `supported.rs::first_unsupported` reports every composable body as `E6001` (`Construct::ComposableFunctions`). Descriptors are two words (`unit.rs::descriptor`). Equality glue exists for aggregates, taking two addresses and returning an `i8` (`glue.rs::glue_signature`), the signature a slot type record's equality entry has.

**Baseline update (`standard-library`, applied 2026-10-18).** Names and mechanisms that differ from what this design assumed:
- The catalog is `intrinsics::CATALOG: &[Intrinsic]`, entries of functions and properties keyed by (module, receiver type constructor, name) with a `BuiltinFunction`, the canonical declaration text (visibility, modifiers, explicit `: Unit`, no defaults) and a `rule_typed` flag; `intrinsics::lookup` finds one. It has no type entries: intrinsic structs need a list of their own.
- Rule-typed intrinsic calls are dispatched in `checker.rs::infer_call_expr` through `intrinsic_callee`, which resolves the callee name as any name (locals, the file, imports, then the prelude) and matches its `BuiltinFunction`; `remember` with keys gets an arm there. An intrinsic named as a value is `E4013`.
- Members are `Extension` records with a `builtin` and a `property` flag, in the checker's `members` table and the shared `PreludeScope`, found by `TypeChecker::member`. Adding `/prism/compose/runtime` to `PRELUDE` needs no other change; `library_tests.rs` checks that prelude names stay distinct.
- Runtime entry points are looked up through `prism_codegen::externals::entry_point` (see `compose-runtime`'s baseline update), not `UnitContext::entry_point`.
- Library units are never reloaded under `--watch` (`standard-library` R3).

**Baseline update (`ui-toolkit` revision, user direction 2026-10-19).** The compose modules follow Jetpack Compose's artifacts, all files of the package directory `prism/compose/` (`ui-toolkit` D21). The runtime module this change creates is therefore `/prism/compose/runtime` (`crates/prism-library/prism/compose/runtime.prism`, `package prism.compose`), still the one compose module in the prelude, and the ABI names its types `prism/compose/runtime.Composition`; every mention below uses that path. It is the library's first module in a subdirectory, so task 2.1 also makes `MODULES`, the embedded file list that `library_tests.rs` compares with `crates/prism-library/prism/` (walked recursively) and the library's sibling imports cover `prism/compose/`. The toolkit modules `/prism/compose/ui`, `foundation`, `layout` and `material3` are `ui-toolkit`'s, outside the prelude. Modifiers are plain data there and never run composition (`ui-toolkit` D25), so this change plans no composed-modifier mechanism.

**Baseline update (apply, 2026-10-19: `compose-runtime` and `receiver-lambdas` archived or implemented, `compose-hot-reload` proposed).** Reconciled with the main specs and code before group 1:
- **`compose-runtime` is archived**, and the main `compose` spec already has a requirement "Remembering a Value" (the runtime's typed and word slots, pinned through `/prism/composer`'s `rememberInt`). This packet's requirement of that name, the language's `remember`, is renamed "Remembering a Value in a Composable" and stays ADDED, so the runtime's requirement is untouched. Every other header of the `compose` delta is new to the main spec, and the MODIFIED blocks of `code-generation` and `standard-library` match the main text they replace except for this change's own edits, so they stand.
- **Runtime entry points** are found through `prism_codegen::externals::entry_point` (and `UnitContext::runtime_function`, which declares an import from it), not `UnitContext::entry_point`: composer operations call the compose entry points that way (D20). The runtime also has word slots (`has_word`, `word`, `set_word`, `changed_word`) for hand-written code; generated code uses typed slots only. `prism_compose_set_slot` already releases a replaced value at the end of the pass (`composer.rs::store`, which overwrites in place only a slot of the same type that needs no drop), as D8's "different" path and D12's changed keys need, so `prism-compose` gains only `prism_compose_kept_slot`.
- **Restarts** run in a *window* frame over the scope's group (`composition.rs::open_scope`), whose group is the parent and whose `window_scope` is the scope: `prism_compose_kept_slot` reads `window_scope`'s slots while the window is the innermost frame, which is exactly "during a restart and before the restart function starts a group" (D10). A restarted scope's group is still marked invalid when it is matched again, so its `skipping` is false and the force bit is redundant there, but kept: it is what makes a restart independent of how the runtime orders the clearing of the mark.
- **`receiver-lambdas`** is implemented: `Type::Fn { receiver, .. }` keeps the receiver as `params[0]`, so a composable receiver lambda (`compose ColumnScope.() -> Unit`) is a composable closure whose first argument is its receiver, covered by change word bits like any parameter ("the receiver first"), and a context parameter is parameter 0 (receiver-lambdas D9). Nothing below changes for them; fixtures add one.
- **`hot-reload` and `instance-cache`** are implemented: generic records (`record.rs::GenericRecord::of_file`) are built from the MIR that `elaborate_file` returns and encoded by `codec.rs`, and code keys hash sources and ABIs, not MIR. Calling the transform inside `elaborate_file` therefore makes records, check entries and every instance hold composed MIR with no other change to the cache.
- **`compose-hot-reload` D8 corrects D22.** A restart closure has type `(Int) -> Unit` whatever its composable's parameters are, so under `hot-reload` D5 its contract would never change and a reload would repoint it to a new `F#restart` that reads the old scope's slots with a new parameter list. Adopted here: `contract.rs::slot_contract` hashes a restart body's contract together with the contract of the composable it restarts, so the two split together, and D22 says so. The second half of that correction, recomposing a retired scope from its parent with its own slots reset, is runtime work of `compose-hot-reload` and stays there.
- **Error codes.** `E4030` is free (`receiver-lambdas`' `E4034` row records `E4030` to `E4033` as reserved by this change and `state-handles`), so `CompositionInComposable` keeps it.

## Goals / Non-Goals

**Goals:**
- Composables run, with Jetpack Compose's model and surface: positional memoization, `remember`, `key`, skipping of composables whose arguments did not change, and recomposition of only the scopes that were invalidated.
- One linear pass per composable body, before instantiation, with no analysis across files or bodies and no per-instance work.
- Keys that survive formatting, comments, argument edits and inserted calls, so that `compose-hot-reload` keeps state.
- Steady-state passes in which generated code allocates nothing for scopes that skip or whose handlers capture one word.
- Clear seams for `state-handles` and `compose-hot-reload`.

**Non-Goals:**
- State handles, readers and invalidation on writes (`state-handles`), nodes (`ui-toolkit`), recomposing reloaded scopes (`compose-hot-reload`).
- Composable default values, keys of other types than `Int` for `key`, stable shared borrows, effects and subcompositions (D17, D13, D7, `compose-architecture` D25).

## Decisions

### D1. The boundary with the other compose changes (autonomous)

| Concern | `compose-transform` (this change) | Elsewhere |
|---|---|---|
| Runtime protocol | one new entry, `prism_compose_kept_slot` (D10) | `compose-runtime` |
| Calling convention, groups, keys | all of it | `compose-hot-reload` relies on the keys |
| Skipping and restarting | stability, change words, kept arguments, restart closures | the runtime decides `skipping` and runs restarts |
| Slot type records and equality glue | emitted per unit | fingerprints from `hot-reload` |
| `/prism/compose/runtime` | `Composition`, `composition`, `setContent`, `recompose`, `dump`, `remember`, `key`, `RecomposeScope`, `currentRecomposeScope`, `invalidate` | `state-handles` adds `State<T>`, `MutableState<T>`, `mutableStateOf` |
| Intrinsic structs | the declaration form and its catalog | `state-handles` declares its handle types with it |
| Reads and writes of state | none | `state-handles`, through the runtime's current scope |
| Nodes | none | `ui-toolkit`'s `Node` intrinsic composable |

Rejected: leaving `currentRecomposeScope` to `state-handles` (every fixture of restart scopes here would then need state); creating `/prism/compose/runtime` with the state types empty (`standard-library` D14 rejects stubs).

### D2. A MIR pass after borrow checking, before records and instances (confirms `compose-architecture` D3, revised)

The transform is a new crate, `crates/prism-compose-transform`, with one entry point, `compose_file(file: &MirFile) -> MirFile`, called by `crates/prism-cli/src/workspace.rs::elaborate_file` right after `prism_borrowck::check_file`. It turns `MirPhase::Elaborated` into the new `MirPhase::Composed`. Every consumer after it reads composed MIR: instance requests and generic records (so a generic composable is transformed once and its instances substitute composed bodies), the build cache's records, and the backend. `prism check` runs it too, because records hold composed MIR. `prism dump-mir --elaborated` stops before it, so its snapshots do not change, and `prism dump-mir --composed` prints after it.

The borrow checker never sees composer plumbing: the composer and change words are `Int` arguments that only the transform reads, and no loan reaches the code it adds. Skipping needs the drops that elaboration placed: a skipped body still drops the function values it owns (D11).

Compile-time cost: a body that is not composable costs one flag test, plus a test of each call terminator for the two intrinsics that need a root content adapter (D15), so a file without compose code costs a scan of its terminators. A composable body costs one walk of its blocks with constant work per statement, linear in the body. Nothing is done per instance.

Rejected:
- Transforming each instance after instantiation (`compose-architecture` D3 before its revision): the transform would run once per instance, and an instance's code would depend on a stability decision per type argument.
- Rewriting the typed tree, as Kotlin's compiler plugin rewrites IR: the borrow checker would check the plumbing.
- Generating the protocol only in the backend: nothing could snapshot it.
- Composing only in `prism run`: records would hold uncomposed MIR, and every instance would need the transform again.

### D3. An explicit composer parameter, the pass identifier word (autonomous; confirms `compose-architecture` D4 with `compose-runtime` D3)

Every composable takes the composer after its declared parameters, as an `Int` argument named `$composer` in MIR, and passes it to every composable it calls and to every protocol operation. Its value is the 64-bit identifier of the pass in progress, which each runtime entry validates with one comparison (`compose-runtime` D3).

`compose-runtime` D4 runs one pass at a time per thread, so a thread-local "current pass" would find the same value. It was rejected anyway. Change words have to be passed per call whatever the composer is, so composable signatures change in any case. An explicit composer keeps every call tied to the pass it belongs to, so subcompositions, nested passes and threads (§5 D25) can come later without changing the calling convention. And a composer captured past its pass is a validated misuse, never a silent use of another pass. Jetpack Compose passes `$composer` explicitly too. The parameter costs one register per call, and the thread-local would cost one thread-local access per entry point, which the runtime pays anyway (`compose-runtime` D4).

Rejected: a thread-local current pass with unchanged signatures (hides which composition a call belongs to, and makes one pass per thread a permanent rule); passing the composition handle (`compose-runtime` D3).

### D4. Change words: two bits per parameter, 31 parameters per word, a force bit (autonomous; revises `compose-architecture` D4's "per 32 parameters")

After the composer, a composable takes `max(1, ⌈n / 31⌉)` change words for its `n` parameters, the receiver first, named `$changed`, `$changed1`, ... in MIR. Parameter `i` has two bits in word `⌊i / 31⌋`, at bits `1 + 2j` and `2 + 2j` for `j = i mod 31`. Bit 0 of the first word is the **force** bit, and every other bit is zero. The two bits say:

| Bits | Meaning | Set by the caller when |
|---|---|---|
| `00` | uncertain | it knows nothing (the default) |
| `01` | same | it forwards its own parameter, which it was told or found unchanged |
| `10` | different | it forwards its own parameter, which it was told or found changed |
| `11` | static | the argument is a constant, or a copy of a top-level `val` |

The force bit is set by restart closures (D10) and by root content adapters (D15), and makes a skippable body run. `compose-hot-reload`'s forced pass reaches bodies through the runtime's `skipping`, which is then false, and needs no bit.

The caller computes each argument's bits with a block-local map, kept while the transform walks a block, from locals to what they copy: a constant, a top-level value, or parameter `p` of the caller. An argument that is not a plain copy of one of those is uncertain. A forwarded parameter gets the caller's own knowledge of it: the bits it was passed, or, in a skippable caller after its comparisons (D8), same or different. The word is then a constant when every argument is uncertain or static, and otherwise built with `Rvalue::Bits` operations (D19).

Defaults stay caller-evaluated generators (`mir-lowering` D11), so no default mask is passed: a defaulted argument is the generator's result, uncertain.

Compile-time cost: constant work per argument of a composable call, and a map cleared at each block. Run-time cost: one register or stack word per word, and a few bit operations per forwarded argument.

Rejected:
- One word per 32 parameters with a separate force argument: one more argument on every call for a bit that fits.
- Jetpack Compose's three bits per parameter: its third bit carries stability known only at run time, and here a parameter type's stability is static (D7).
- A default mask (§5 D4).

### D5. One group per composable body, keyed by its identity; groups for regions; no call groups (autonomous; revises `compose-architecture` D5)

Every call of a composable runs the callee's body inside one group that the callee starts first and ends before it returns: a restart group when the composable is skippable, a replaceable group otherwise. A call adds no group of its own. A region that contains composable calls runs in a replaceable group of its own (D6): a branch of `if` or `when` (the guard included), the right operand of `&&` and `||`, the fallback of `?:`, the part of a safe call after its test, and a loop, around all its iterations.

Keys are the first 64 bits of a BLAKE3 hash, chained so that each key costs one hash of a few dozen bytes:

- **a composable function's body**: `H("body", declaration path)`, the path as the ABI writes it (`app/main::Counter`, `app/main::User.Avatar`);
- **a composable closure's body**: `H(parent, "closure", <callee path>, <ordinal of that call among the calls of the same callee in the parent region>, <parameter index>)` for a closure written as an argument, and `H(parent, "closure value", <ordinal among such closures of the parent region>)` otherwise. `parent` is the key of the region the closure is written in, or `H("body", declaration path)` of the enclosing top-level declaration when its body is not composable;
- **a region**: `H(parent, <construct>, <ordinal among sibling constructs of that kind in the parent region>, <branch index>)`, with constructs `if`, `when`, `and`, `or`, `elvis`, `safe call` and `loop`;
- **a `key` group**: `H(parent, "key", <ordinal among the key calls of the parent region>)`, with the `Int` value as its data key (D13).

Keys use declaration paths, construct kinds and ordinals, never offsets, lines or columns. A comment, a reformatting, an edited argument or an inserted call of another composable changes no key. Inserting a construct of the same kind before another shifts the later ordinals, and with them that state, which is the positional behavior Jetpack Compose has.

Calls of the same composable among the same siblings have equal keys, so the runtime matches them in order (`compose-runtime` D5): that is the ordinal of §5 D5, at no cost. A restart recomposes a group whose key is the callee's own constant, so a restart function needs no key (D10). The loop's group surrounds all its iterations rather than each one, so that `key` groups of different iterations are siblings and can be reordered among themselves. A group per iteration would match iteration `i` against old iteration `i` and lose every reordered item's state. Calls in iterations match by key in order, as in Jetpack Compose, which groups no iteration either.

The instances of a generic composable share its body key, because keys are computed once, before instantiation (D2). Calls of `Show<Int>` and `Show<Bool>` among the same siblings match in order. When their order changes, a group is matched with an instance of another type, its slots fail the type check and are recreated, at the cost of that state only.

Compile-time cost: one hash per group, from its parent's key and its own step, so linear in the number of groups whatever the nesting depth; one counter per construct kind per open region for ordinals. Run-time cost: one group per composable call, where per-call groups would cost two.

Rejected:
- A group per call keyed by the caller's lexical path, as §5 D5 first planned: two groups per call, and the restart function would have to capture or be passed the call's key.
- A `$key` parameter from the caller: one more argument on every call, for what in-order matching already gives.
- Source offsets (Jetpack Compose): every edit above a call changes them.
- A group per loop iteration: it defeats `key` (above).
- Hashing the whole lexical path at every group: O(depth) per group, quadratic for deep nesting.

### D6. MIR lowering marks regions; a linear pre-pass decides which regions contain composable calls (autonomous)

Only lowering knows a body's source structure and every edge that leaves a construct: falling through, `return`, `break` and `continue`, which the scope machinery already follows to place drops (`build/scope.rs`). So lowering marks regions, and the transform turns marks into protocol calls:

- `Body::groups: Arena<GroupId, GroupRegion>` holds, per region of a composable body, its parent region, its construct step (D5) and its span. Lowering a composable closure records `Body::origin: Option<ClosureOrigin>`, the region of its parent and the step that reaches it.
- `StatementKind::GroupStart(GroupId)` begins a region, and `StatementKind::GroupEnd(GroupId)` is emitted on every path that leaves it. Regions are pushed with the scope that holds them, so exiting scopes emits group ends in the order it emits drops, innermost first.
- Before lowering a composable body, `build/groups.rs::composable_regions` walks its typed tree once, post-order, and records which expressions contain a composable call outside closures, the intrinsic composables included. Lowering opens a region only for a construct that contains one.

Refined at apply time: a region is attached to the scope lowering already opens for its construct (a branch's temporary scope, a loop statement's scope, a `?.` chain's scope), so `exit_scope` emits its end after that scope's drops on every exit, and `break` and `continue`, which leave only the scopes inside a loop, never end the loop's region. A `when`'s region holds its branch body, not its guards or later conditions, which are evaluated before the branch is chosen and stay in the enclosing group; a `?.` chain's region holds the whole chain, its receiver included, ending on the `null` exits too. Ordinals count only regions, so inserting an `if` that composes nothing shifts no key.

Markers read, write and drop nothing. The borrow checker, drop elaboration and liveness skip them with one match arm each, and `prism dump-mir` prints them, so the lowering of regions is pinned by `mir.snap` fixtures before the transform runs. The fixture "Composable calls are marked" has no region, and its snapshot does not change.

Compile-time cost: one bit per typed-tree expression of a composable body, then one marker per region exit, which the scope machinery already walks for drops.

Rejected:
- Recovering regions from the control-flow graph in the transform, with dominators and post-dominators: more work, and the construct kinds that keys need are lost.
- Groups for every branch of a composable body: run-time cost for branches that compose nothing.
- Side tables of block ranges: blocks are merged and renumbered after lowering (`merge_goto_blocks`).

### D7. Stability by declared markers; function values keepable at run time (autonomous; refines `compose-architecture` D9)

A parameter, the receiver included, is **stable** when:

- its type is `Copy` and is not a reference: by the declared `Copy` marker of a struct or enum (read from the file's type table, which holds imported definitions with their markers), the built-in rules for scalars, `Range`, `T?`, intrinsic structs such as `RecomposeScope`, and for a type parameter only its `Copy` bound. Type arguments are borrow-free (`E4022`), and struct fields cannot be borrows, so no other borrow check is needed;
- or its type is an `escaping`, non-composable function type, or a nullable of one.

A composable is **skippable** when it returns `Unit` and all of its parameters are stable. Composable function types (content lambdas), `String`, references, unmarked function types and unbounded type parameters make it not skippable, as §5 plans. A shared receiver is a reference, so an extension composable is skippable only with a `consuming` receiver of a stable type. Stable shared borrows of `Copy` values, kept as copies of the referent, are a follow-up.

A function value is **keepable** when its closure captures nothing or owns only `Copy` captures, which its descriptor says (D9). Whether an `escaping` argument is keepable is known only at run time: `{ count.value += 1 }` is, `{ println(name) }` owning a `String` is not. So a skippable composable with function parameters tests, before starting its group, whether every function argument is keepable (one load and compare each). If one is not, it runs that call as a composable that is not skippable, in a replaceable group with the same key, which never matches its restart group, so no slot is misread. A call site that always passes the same closure literal always takes the same path.

Compile-time cost: one marker lookup per parameter type, memoized per file, and no analysis of fields, bodies or other files. Kotlin infers stability from fields across modules and needs `@Stable` annotations and configuration files where that fails; here the declared `Copy` marker plays their role, which Prism already requires for copying.

Rejected:
- Inferring stability from fields (whole-program, and skipping would depend on other files' private fields).
- Treating every function parameter as unstable: §5's handler scenario, and most UI event parameters, would never skip.
- Rejecting unkeepable closures for stable parameters: a handler owning a `String` is legitimate.
- Shared borrows of `Copy` values as stable now (the follow-up above).

### D8. Kept arguments: slots in parameter order, compared through equality glue (autonomous)

A skippable body keeps its arguments in the first slots of its restart group, in parameter order: a `Copy` value by copy, a function value as a duplicate (D9), so the body keeps the original. For each parameter, by its change bits (D4):

- **uncertain**: `changed(composer, record, value)` stores the value and reports whether it differed from the kept one by the record's equality entry, the type's equality glue (§5 D16);
- **different**: `set_slot` stores it, and the parameter is changed;
- **same** or **static**: `Remembered` (D19) reads past the kept slot; only when there is none, on the group's first composition, the value is stored. The parameter is unchanged.

The body then knows, per parameter, same or different, and forwards that to its own calls (D4). Equality is the type's equality glue, called through the record (D20), whatever the type's size: no comparison is expanded at the call site.

Run-time cost: one runtime call per stable parameter per call, plus one equality glue call for uncertain ones; no allocation, except when an uncertain function argument's environment is allocated, whose duplicate is allocated and, when equal, freed.

Rejected:
- Comparing in generated code at each call site: code per site, super-linear in large types (§5 D16).
- Keeping the arguments in the restart closure's environment: an allocation each time a scope with more than one word of arguments ends (`compose-runtime` D8), and a second duplicate of every function value.
- A compare-without-storing entry, to avoid duplicating equal heap environments: an optimization to measure first.

### D9. Keepable closures: four-word descriptors with environment equality and duplicate entries (autonomous)

A descriptor becomes four words: the slot, the drop entry, the **environment equality entry** `fn(environment word, environment word) -> u8`, and the **environment duplicate entry** `fn(environment word) -> environment word`.

| Closure | Equality entry | Duplicate entry |
|---|---|---|
| captures nothing (also adapters) | `prism_rt_empty_environments_equal`, a runtime function returning true | null: the null word is copied |
| inline environment, owning only `Copy` captures | `<closure>#equals-environment`, comparing the captures held in the two words with `==` | null: the word is copied |
| allocated environment, owning only `Copy` captures | `<closure>#equals-environment`, comparing the captures with `==` in capture order | `<closure>#duplicate-environment`: allocates and copies the captures |
| borrowing, or owning a capture that is not `Copy` | null | null |

A function value is keepable exactly when its equality entry is not null. Duplicating it copies the descriptor address and, when the duplicate entry is null, the environment word, and otherwise calls the entry. The equality glue of a function type (D20) compares descriptors, then calls the equality entry. Captures are compared with `==` of their types, through equality glue for aggregates, so `0.0` and `-0.0` are equal and `NaN` is not, as for any `==`.

Compile-time cost: one small function per closure that owns only `Copy` captures, two when its environment is allocated, and nothing for borrowing closures or closures capturing nothing, which are most closures outside composables.

Rejected:
- Comparing environments bitwise: `==` of `Double` and of struct padding would differ.
- A `Copy` function type marker, a language feature (`escaping-closures` D11 follow-up).
- One equality function per environment layout, shared by closures: fewer functions, but it needs layouts interned across a unit, a follow-up if code size asks for it.

### D10. Restart functions are static closures that read the kept slots (autonomous)

A skippable composable `F` gets one more closure body, `F#restart` (`BodyOwner::Restart`), of type `(Int) -> Unit`, capturing nothing. It reads each kept argument from the slots of the scope's group with `KeptParameter { index, type }` (D19), calls `F` with them, the composer it was given, and change words with the force bit and every parameter same, and returns. `F` ends its restart group with the value of `F#restart`: a static descriptor and a null environment, so ending a restart group allocates nothing, and replacing a restart function with an equal one releases nothing.

Reading the slots before `F` starts its group needs one new runtime entry, `prism_compose_kept_slot(composer, index, type) -> *const u8`: during a restart and before the restart function starts a group, the address of slot `index` of the scope's group when it holds `type`, and otherwise the misuse `no slot to read` (compose, "Reading the Kept Slots of a Restarted Scope"). `compose-runtime` D8 anticipated this ("`compose-transform` can avoid it by reading kept parameters from slots"). The restart frame already knows the scope's group, so the entry costs one lookup. A kept `Copy` value is copied out, and a kept function value duplicated.

`F` then starts its restart group, which matches the scope's group (`compose-runtime` "Recomposing a Scope Alone"), reads past its kept slots since every parameter is same, and runs its body because of the force bit.

Run-time cost: one slot read per parameter per restart. Compile-time cost: one small body per skippable composable, linear in its parameters.

Rejected:
- Capturing the arguments in the restart closure (D8, rejected there).
- A separate `F#body` function called by both `F` and the restart: one more function per composable and one more call per composition.
- Calling `F` with dummy arguments and a restart bit, reading the slots inside: MIR would need zero values of every type.

### D11. The shape of a composed body

A skippable body, schematically (the dump of D19 prints it as MIR):

```
F(params..., $composer, $changed...):
    if any function argument is not keepable: goto replaceable path      (D7)
    start_restart_group($composer, key(F))
    for each parameter p: dirty[p] = keep(p)                            (D8)
    if !force && no dirty[p] && skipping($composer):
        drop the function arguments it owns
        skip_to_group_end($composer); goto end
    body, with every composable call passing $composer and change words (D4)
end:
    end_restart_group($composer, F#restart)
    return
```

The replaceable path is the body of a composable that is not skippable: `start_group($composer, key(F))`, the body, `end_group` before each return, its calls forwarding the bits it was given. Region markers become `start_group(key)` and `end_group`. Returns inside regions already end their regions (D6), and the transform puts the body group's end before every `Return` terminator. A composable returning a value writes `_0` before its group ends.

The skip path drops only function values, the only stable arguments that need a drop. Nothing is moved before it, so the drops are unconditional, and no drop flag is involved.

The replaceable path duplicates the body's blocks, which would double a large skippable body with function parameters. It is shared instead: the body's blocks are emitted once, after a join where a `Bool` local records whether the restart group was started, and the body group's end switches on it. Only the start and the end differ.

### D12. `remember`: an intrinsic composable over the current group's slots; up to three keys (autonomous)

`remember<T : Copy>(calculation: () -> T): T` is a signature-typed intrinsic composable of `/prism/compose/runtime`, and with keys a rule-typed one (`standard-library` "Intrinsic Calls Are Typed by Their Signatures"): up to three keys before the calculation, each typed by a fresh type parameter bounded `Copy`, so a non-`Copy` key is `E4021` and a borrowed one `E4022` by the existing checks. The calculation's parameter is not `escaping`, so `remember { 7 }` borrows its captures and allocates nothing (`escaping-closures` D12).

It adds no group: its slots are in the current group, after any kept arguments, in source order. It lowers to:

```
changed = false
for each key k: changed |= changed(composer, record(K), k)    // keys always advance the cursor
if !changed: Remembered(record(T)) → found: result = slot value; done
result = calculation()                                       // missing slot, or a key changed
set_slot(composer, record(T), result)                        // forgets the old value
drop the calculation value                                    // the callee would have dropped it
```

A changed key's `set_slot` forgets the old value, which is released at the end of the pass (`compose-runtime` D7). `T : Copy` is §5 D8: a non-`Copy` value lives in a remembered state cell (`state-handles`).

Compile-time cost: constant per call. Run-time cost: one runtime call per key, one slot read, and on a miss the calculation and one write.

Corrected at apply time: a key written as a reference (`remember(r)` with `r: &Int`) is not `E4022`. Like any argument of a type parameter, it is copied out of the reference when its referent is `Copy` (references, "Copy-Out From References"), so the key is an `Int`. `E4022` stays for a key whose type holds a borrow after that, such as `(&Int)?`, which the scenario now uses.

Rejected:
- Kotlin's `vararg` keys: no `vararg` yet, so up to three, as in Kotlin's common overloads.
- `remember1`, `remember2`: names that Kotlin users would not look for.
- A group per `remember`: a group per memoized value for nothing that slot order does not give.

### D13. `key(value: Int) { }` (autonomous)

`key` is an intrinsic composable that starts a movable group with its region key (D5) and `value` as its data key, calls its content with the composer, and ends the group. The content is an ordinary composable lambda (D14), lent, so it allocates nothing.

Only `Int` values: the runtime matches movable groups by their exact 64-bit data key (`compose-runtime` D5), which an `Int` is. Other types would need a hash, and two items whose keys collide would silently share state. The todo app's ids are `Int`.

Rejected: any `Copy` type through a structural 64-bit hash (collisions); `String` keys (a hash, and not `Copy`); several keys (`key(a, b)`, a follow-up with traits).

### D14. Composable lambdas and composable function values

A composable closure's code takes `(result?, environment, parameters..., $composer, $changed...)`, and its body runs in a replaceable group keyed by D5. It is never a restart scope (§5 D10): a content lambda borrows its author's frame, so a state read in it invalidates the nearest restart scope above it, which reruns the author, and the author calls the lambda again. A call of a composable function value (`content()`, `item(i)`) passes the caller's composer and change words for the value's parameters (D4), through `Callee::Value` like any call of a function value.

A trailing lambda such as `Column { Text("a") }` is a closure checked against `compose () -> Unit`, so it is a composable closure, lent to `Column`, allocating nothing (`closures` D5).

### D15. Entering composition: an owned `Composition`, root content adapters, and `currentRecomposeScope` (autonomous)

`/prism/compose/runtime` (`crates/prism-library/prism/compose/runtime.prism`, `package prism.compose`, added to `PRELUDE`) declares, all as intrinsics:

```
public intrinsic struct Composition
public intrinsic struct RecomposeScope : Copy
public intrinsic fun composition(content: escaping compose () -> Unit): Composition
public intrinsic fun Composition.setContent(content: escaping compose () -> Unit)
public intrinsic fun Composition.recompose(): Bool
public intrinsic fun Composition.dump(): String
public intrinsic compose fun <T : Copy> remember(calculation: () -> T): T
public intrinsic compose fun key(value: Int, content: compose () -> Unit)
public intrinsic compose fun currentRecomposeScope(): RecomposeScope
public intrinsic fun RecomposeScope.invalidate()
```

- **Root content adapters.** The runtime calls root content as an `(Int) -> Unit` with the composer (`compose-runtime` D8), while `content` is a `compose () -> Unit`, whose code also takes a change word. A body calling `composition` or `setContent` therefore gets one adapter closure, `<symbol>#root-content` (`BodyOwner::RootContent`), owning a capture of type `compose () -> Unit` and calling it with the composer and the force bit. Each call builds the adapter from its content, moved in, which allocates its 16-byte environment once per composition or content change, freed by `#drop-environment` when the runtime releases the root content.
- **Receivers.** `setContent`, `recompose` and `dump` take a shared receiver: a composition's state lives in the runtime behind its handle, so `val c = composition { }` works as in Kotlin, and a `mut` receiver would require `var`.
- **`currentRecomposeScope()`** is Jetpack Compose's `currentRecomposeScope`, a call because Prism has no properties outside the library's intrinsic extension properties. With `invalidate()`, it lets this change's fixtures invalidate scopes before `state-handles` exists, and it stays useful afterwards, as in Kotlin.
- **Disposal** is the drop glue of `Composition` (D16). A composition is disposed when its owner's scope ends, which `--check-allocations` checks.

Rejected:
- One global composition started by `runApp` (§5 D11).
- Exposing the composer or composition words, as `/prism/composer` does: application code would see the protocol.
- `setContent` and `recompose` as `mut` members (above).

### D16. Intrinsic structs (autonomous; brings forward the intrinsic types that `standard-library` D6 left to `state-handles`)

`intrinsic struct Name<Parameters> : Markers` declares, without fields, a type whose representation, glue and operations the compiler provides. The intrinsic catalog gains types: `prism/compose/runtime::Composition` (one word, neither `Copy` nor `Clone`, its drop glue calling `prism_compose_dispose`) and `prism/compose/runtime::RecomposeScope` (one word, `Copy`). A library declaration of another name, or with other type parameters or markers, is `E4029`, and a workspace one `E3002`, as for other intrinsics. No program can construct one, read a field or take one apart: their intrinsic functions make and use them. `==` compares the handle word.

`Composition` must be owned and disposed by its drop (§5 D11), and no Prism declaration can give a type a drop that calls the runtime: user-defined destructors would be a language feature of their own. `state-handles` needs the same form for `State<T>` and `MutableState<T>`, so it is added once, here.

Compile-time cost: one catalog lookup per intrinsic struct declaration, in the library's own check, cached.

Rejected:
- User-defined destructors (`fun Composition.drop()`): a language decision far beyond this change.
- Compiler built-in types without declarations: the type-checker tables that `standard-library` removed.
- A library struct with an `Int` field: forgeable, and no drop.

### D17. What is rejected (autonomous)

- **`E4030` (`CompositionInComposable`)**: `composition`, `setContent` and `recompose` called in a composable context. The runtime makes them misuse during a pass (`compose-runtime` D4), and a call in a composable context always runs during one. A check of the compose-context flag at each call of these three intrinsics, constant per call. Calls in a handler written inside a composable are accepted, since handlers run outside passes.
- **`E4008` for composable calls in default values.** A default's generator runs in the caller before the call (`mir-lowering` D11), outside the callee's groups, and is not a composable body, so `check_function_body_in_scope` checks defaults outside any composable context. Kotlin allows composable defaults (`color = LocalContentColor.current`), which need composer parameters on generators and groups in the caller. No `CompositionLocal` exists yet, so this is a follow-up. Today a composable default is accepted by `prism check` and rejected by the backend (`E6001`), so no running program changes.
- **`E4021` and `E4022`** for `remember` values and keys (D12), and `E4004` for a `key` value that is not an `Int` (D13).
- **`E6001`** no longer names composable functions: `Construct::ComposableFunctions` is removed. It keeps values of unknown type and conversions the backend does not lower.

What stays a runtime error: dropping a `Composition` during a pass, as a composable owning one would at its end, is `composition misuse: composition operation during a pass` (`compose-runtime` D4). A static rule would need to follow `Composition` through every type that may hold one.

### D18. `/prism/composer` stays public and unchanged (autonomous; answers `compose-runtime`'s open question)

Generated code calls the compose entry points directly from the backend (D20), not through the module's externals, so the transform does not depend on it. It stays as the protocol's hand-testing surface, outside the prelude, and its fixtures keep pinning the runtime from Prism. Hand-written composables in it and real composables cannot call each other: a `compose fun` is never called with an explicit composer, and a plain function cannot call a composable (`E4008`).

Rejected: making its wrappers private or removing it (its fixtures would go, and with them the runtime's Prism-level pins).

### D19. Composed MIR

- **Arguments.** A composed composable body has `argument_count` increased by one plus its change words, named `$composer`, `$changed`, `$changed1`, ..., typed `Int`. Its locals keep their order after them: the return place, the declared arguments, the new arguments, then captures and other locals, so local numbers after the declared arguments shift, which composed snapshots show.
- **`Callee::Composer(ComposerOperation)`**, called with ordinary `Call` terminators: `StartRestartGroup { key }`, `StartGroup { key }`, `StartMovableGroup { key }` (data key as an operand), `EndGroup`, `EndRestartGroup` (the restart function moved), `Skipping` (`Bool`), `SkipToGroupEnd`, `Changed(Type)` and `SetSlot(Type)` (the value moved), `KeptParameter { index, ty }` (the kept value, copied or duplicated), `Keepable` (a shared borrow of a function value, `Bool`), `Duplicate` (a shared borrow of a function value, the duplicate), `CurrentScope` (`RecomposeScope`), and, outside composables, `StartComposition`, `SetContent`, `Recompose`, `Dump` and `Invalidate`, which replace the `Callee::Builtin` calls of those intrinsics.
- **`TerminatorKind::Remembered { composer, slot_type, destination, found, missing }`** reads the slot at the cursor: when it holds `slot_type`, it copies the value to `destination` and continues at `found`, and otherwise at `missing`. It is the one composer operation with two successors, since `prism_compose_slot` both tests and advances.
- **`Rvalue::Bits(BitOperator, Operand, Operand)`** with `And`, `Or`, `ShiftLeft` and `ShiftRight` (logical) on `Int`, without overflow checks, for change words. No source operator lowers to it yet, and source bitwise operators can later.
- **Owners.** `BodyOwner::Restart { function: BodyId }` (symbol `<function symbol>#restart`) and `BodyOwner::RootContent { parent: BodyId }` (symbol `<parent symbol>#root-content`): named by what they belong to, not by a closure index, so inserting a closure in the source never shifts them (`hot-reload` D5).
- **Phase.** `MirPhase::Composed`, which `validate.rs` checks: no region marker, composable bodies with their extra arguments, every composable call passing them, composer operations only in composable bodies except the composition ones, and every body group closed before each return.
- **Dump.** `prism dump-mir --composed` prints `compose` body headers with `composed`, and `skippable` for skippable ones, composer operations as `composer <operation>`, keys as 16 hexadecimal digits followed by their construct in a comment (`key 0x3f9a0c12d4e5b678 /* if 0 then */`), and `Remembered` with its two targets. `--composed --instances` is not a separate flag: `--instances` prints composed instances, since records hold composed MIR.
- **Instances and records.** `instantiate.rs` substitutes types in `Changed`, `SetSlot`, `KeptParameter` and `Remembered`. `record.rs::GenericRecord::of_file` assigns restart and root content bodies to their parents' records. `instance-cache`'s codec encodes every new construct, which its exhaustive matches enforce.

**Refined at apply time.** The transform replays each body's blocks in order into new blocks, splitting a block at each region marker and composer operation, and appends the blocks it adds; the prologue it builds last becomes `bb0` (lowering never jumps to `bb0`, which the transform checks by remapping such jumps), then `merge_goto_blocks` tidies the result. Its temporaries are storage-live from `bb0` to every return, which keeps the validator's storage rules without placing markers per use. Keys carry a label (`GroupKey { hash, label }`) that only dumps read. The ordinal of a `key` call counts the `key` calls of its innermost region in block order, from the open-region stacks the validator also computes (`prism_mir::open_regions`), so lowering records no `key` steps; a closure's step is recorded by lowering, which knows the call and parameter it is written for. A kept function value is duplicated before it is given to `changed` or `set_slot`, so the body keeps the original; a remembered slot read past by `Remembered` lands in a scratch local that is never dropped, since the slot keeps owning the value.

### D20. The backend

- **Signatures.** `representation.rs`'s body signatures take the new `Int` arguments like any others, and `function_value_signature` appends a composer and the change words when the function type is composable. A composable body's hot-reload contract covers them, since it is made of the body's declared argument types.
- **Composer operations** are direct calls of the compose entry points, through `UnitContext::entry_point` (`compose-runtime` D2), with values for slots spilled to a stack slot and passed by address. `Keepable` and `Duplicate` are lowered inline: loads of the descriptor's words and a compare, or a call of the duplicate entry.
- **Slot type records.** `UnitContext::slot_type(ty) -> DataId` defines `<unit>#slot-type(<type>)` once per type: the type's fingerprint from `TypeFingerprints`, its layout's size and alignment, its drop glue address or null, its equality glue address, and its name, written as glue names it, in the unit's data. Records of one type in two units are different records with the same fingerprint, which the runtime matches by fingerprint (`compose-runtime` D6).
- **Equality glue for every kept type**, scalars included (`#equals(Int)` compares two loaded words), and for function types (D9). Strings never need it: they are not stable and not `Copy`.
- **Intrinsic structs** are laid out as one pointer-sized word, `Composition`'s drop glue calls `prism_compose_dispose`, and `dump-layout` names them `prism/compose/runtime::Composition` and `prism/compose/runtime::RecomposeScope`.
- **Descriptors** of four words (D9), with the environment functions defined in the closure's unit. Refined at apply time: `prism_runtime::Descriptor`, the Rust view, keeps its two fields, the prefix the compose runtime and `prism-ui` read to call and release function values; only generated code reads the last two words, and only from values generated code built.
- **Intrinsic struct layouts.** An intrinsic struct is laid out as a struct with one hidden `Int` field, `handle`, so copies, `==` and equality glue compare the handle with no special case; `dump-layout` shows that field.
- **The phase.** A unit of MIR that is not composed is an internal error, and so is a region marker reaching the lowering.
- **`E6001`.** `supported.rs` stops reporting composable bodies, and a body that reaches the backend with region markers or in another phase than `Composed` is an internal error.

Compile-time cost: one record per type a unit keeps or remembers, from fingerprints memoized per load, and the glue functions already memoized per unit and type.

### D21. Generic composables are composed once

The transform runs on a generic composable's polymorphic body (D2): its stability uses bounds (D7), its keys its declaration path (D5), and its slot types mention its type parameters. Its record holds the composed body with its restart and root content closures, and each instance substitutes them (D19), so slot type records are emitted per instance unit for concrete types (D20). Nothing about composing a body depends on its type arguments. A `T` parameter without a `Copy` bound makes the composable not skippable, which a `T : Copy` bound fixes, the explicit form §5 D3 accepted.

### D22. Seams for hot reload

- Keys depend on declaration paths and structure, never on positions, so a reloaded body keeps the state of every group whose construct path did not change (D5).
- A restart closure is in its composable's unit and named after it, so `compose-runtime`'s `invalidate_where`, given the slots of the reloaded units, finds every restart scope of a reloaded file (§5 D23).
- When a kept parameter's type changes, the composable's contract changes and it gets a new slot (`hot-reload` D5). A restart closure's own type never changes, so its contract is hashed together with its composable's (`compose-hot-reload` D8, adopted at apply time): `F#restart` splits with `F`, and old restart closures keep calling the old code, which reads its own slots with its own records. New code installs its restart closure only after starting its own group, whose kept slots it has just written, so `prism_compose_kept_slot` is never asked for a slot of another type by code that wrote it. Recomposing such a retired scope from its parent, with its slots reset, is `compose-hot-reload`'s.
- `compose-hot-reload` recomposes reloaded scopes with a forced pass, through `skipping` (D4), and needs no bit of its own.

### D23. Seams for `state-handles`

- **Reads** register the innermost restart scope of the active pass, which the runtime knows (`compose-runtime` D1), including reads in content lambdas and in plain functions called from composables: the restart scopes are exactly the skippable composables (D7) and the root.
- **`remember { mutableStateOf(...) }`**: the calculation runs as the only code between the decision to compute and the `set_slot` of its result (D12), and it is not a composable context, so no group or slot operation of the composer happens in between. `state-handles` attaches the cells created in that window to that slot, adding to the lowering of D12 whatever runtime calls mark the window.
- **Static rules**: the compose-context flag, which this change keeps for defaults (D17), is what `state-handles`' errors for writes and `mutableStateOf` in composable contexts read.
- **Handles** are intrinsic structs (D16), `Copy`, so they are stable parameters, and a handler capturing one has a one-word inline environment, keepable without allocation (D9).
- **Equal writes** use the same equality glue that kept arguments use (D20).

### D24. Costs, and how they are measured

| Stage | Compile-time cost |
|---|---|
| region pre-pass (D6) | one bit per typed-tree expression of composable bodies |
| region markers (D6) | one marker per region exit, along the scope exits lowering already walks |
| transform (D2) | one walk per composable body, constant work per statement, argument and group; a terminator scan for other bodies |
| keys (D5) | one BLAKE3 hash of a few dozen bytes per group |
| stability (D7) | one marker lookup per parameter type, memoized per file |
| restart and adapter bodies (D10, D15) | one small body per skippable composable and per body starting a composition |
| backend (D9, D20) | one record per kept type per unit; one or two small functions per closure owning only `Copy` captures |

Nothing is per instance or whole-program. `crates/prism-cli/tests/compile_timing.rs` gains five shapes, at its usual sizes: `composable-calls` (one composable calling `n` skippable composables), `composable-branches` (`n` sequential `if`s each calling a composable), `nested-composable-branches` (nesting depth `n`), `many-composables` (`n` skippable composables of four stable parameters) and `composable-lambdas` (`n` content lambdas). `PRISM_ASSERT_TIMING=1` asserts that each grows roughly linearly.

Measured at apply time (release build, best of five, ms, at the largest size; the `compose` column is the transform, which `compile_timing.rs` now times as a stage of its own between `borrow` and `inst`):

| shape | size | borrow | compose | lower | clif | worst growth |
|---|---|---|---|---|---|---|
| `composable-calls` | 4096 calls | 5.5 | 3.0 | 4.1 | 5.2 | x4.0 per x4 |
| `composable-branches` | 4096 `if`s | 23.5 | 19.6 | 22.3 | 29.9 | x5.0 per x4 (`mir`) |
| `nested-composable-branches` | depth 120 | 0.4 | 0.3 | 0.3 | 1.5 | below 5 ms |
| `many-composables` | 4096 composables | 31.5 | 100.8 | 359.2 | 871.7 | x5.1 per x4 (`mir`) |
| `composable-lambdas` | 4096 lambdas | 39.8 | 19.0 | 40.6 | 117.4 | x4.5 per x4 |
| `nested-compose-ui` (now compiled) | depth 128 | 48.7 | 17.4 | 59.7 | 152.3 | x1.93 per x2 on average |

Every stage grows roughly linearly. The transform costs about the borrow checker's time or less except for `many-composables`, where each skippable body's prologue keeps four arguments (one switch over four bit patterns each) and its code is what the backend then spends most on: about 0.2 ms of `lower` and `clif` per composable of four parameters, linear. Sharing that prologue in a runtime helper is a follow-up if large UIs show it.

| Run-time operation | Cost |
|---|---|
| composable call | one group start and end, one runtime call per stable parameter, change words |
| skipped call | as above, plus `skipping` and `skip_to_group_end`; no allocation for `Copy` arguments and inline handlers |
| restart | one kept-slot read per parameter, then a call |
| `remember` | one runtime call per key and one slot read |

### D25. Capabilities and tests

The language side of compose extends the `compose` capability that `compose-runtime` creates, so that one spec describes what a composable does. MIR forms extend `mir`, backend forms `code-generation`, the defaults rule `type-system`, the module and intrinsic structs `standard-library`, `syntax`, `compilation-and-abi` and `formatting`. The diagnostics catalog is edited directly.

Behavior is pinned by executable fixtures under `crates/prism-cli/tests/fixtures/compose/`, all under `--check-allocations`, most printing what runs on each pass and some printing `c.dump()`. The transform's output is pinned by a new snapshot kind, `composed-mir.snap` (`prism dump-mir --composed`), required for the `mir` scenarios of composed MIR. Key stability under edits is a CLI test comparing two composed dumps. The kept-slot entry is a Rust test in `prism-compose`.

### D26. Apply-time refinements (autonomous)

Decided while implementing; each is also noted where its decision is:

- **A `when`'s region is its branch body** (D6): guards and later branch conditions run before the branch is chosen and stay in the enclosing group. Rejected: a region around the whole `when` (one group for every branch, losing the per-branch state switch), or regions that start before a guard (a guard's false edge would have to end a group it may never have started). A composable call in a guard is rare and still type-safe through typed slots.
- **`key` ordinals count the `key` calls of their innermost region in block order** (D5, D13), read from the open-region stacks the validator computes. Rejected: recording a `key` step in lowering, which would need a marker or a side table that survives block merging.
- **The Rust `Descriptor` stays a two-word prefix** (D9): only generated code reads the equality and duplicate entries, from values generated code built. Rejected: four Rust fields, which would change the test descriptors of `prism-compose` and `prism-ui` for words they never read.

## Risks / Trade-offs

- **[Positional matching of same-callee calls]** Inserting a call of the same composable before another shifts their state, as in Jetpack Compose. → `key` gives identity where it matters (lists).
- **[Shared keys of generic instances]** Instances of one generic composable among the same siblings match in order, and a reorder resets their state. → Slot types make it safe (D5, D21), and the case is rare.
- **[Dynamic restartability]** A skippable composable called with an unkeepable handler runs in a replaceable group, and a call site that alternates between keepable and unkeepable closures recreates its group each time. → Each call site passes one closure literal in practice, and the dump shows the group kind.
- **[Composed snapshots move local numbers]** The new arguments renumber later locals in composed dumps only. → Elaborated snapshots are unchanged.
- **[The descriptor grows to four words]** Every closure's static data grows by 16 bytes. → Static data only; function values stay 16 bytes.
- **[Default values of composables]** Kotlin code that uses composable defaults does not port directly. → `E4008` with a note, and a follow-up.
- **[`Composition` dropped during a pass]** A runtime misuse, not a compile error (D17). → A composable rarely owns a composition, and the error names it.

## Migration Plan

No program that runs today changes meaning. Composable programs, which the backend rejected, now run. A composable call in a composable's default value, which `prism check` accepted, is now `E4008`. `prism dump-clif` and `prism dump-mir --instances` print composed code. `prism check --explain-cache` lists `prism/compose/runtime.prism`, and its snapshots are re-blessed and read against "Library Modules Compile Like Workspace Files". The `E6001` fixtures that used composables move to unknown types, as the MODIFIED scenarios say.

## Open Questions

None blocking. The follow-ups (composable defaults, stable shared borrows of `Copy` values, keys of other types for `key`, a compare-without-store entry, shared environment equality functions) go to `ROADMAP.md` at archive.
