# Design: compose-library-in-prism

## Context

See `proposal.md` for the motivation. This change is item 10c of "After Milestone 2: the review round" in `ROADMAP.md`, the compose half of agreed item **A4** of `docs/milestone-2-review-notes.md`:

- "as much as possible ... normal prism code that is imported like any other code" (user, 2026-10-20);
- a minimal core with "one composer-access primitive";
- no `unsafe` for now (user decision).

`core-library-in-prism` D10 classified the compose declarations, recommended what to move, and asked this change to challenge the classification where Compose itself uses library code over `currentComposer`.

The user is away. Every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Decisions the user took in the review are marked **(user direction 2026-10-20)**. Compile speed is a primary criterion (AGENTS.md §5), so each decision states its compile-time and run-time cost.

**Baseline.** Proposed on top of `8b4a053` ("Propose field-visibility"), with these packets landing before this one, in `ROADMAP.md`'s order:

- `drop-and-clone-hooks` (item 8, being archived): hooks, which `Composition` uses (its Composition group already checked that a hook can dispose a composition when a released state is freed).
- `vararg` (9, proposed).
- `core-library-in-prism` (10a, proposed): extension properties, `/prism/core` as the only member module, members written in Prism. Two blocks of this packet are written on its versions:
  - `standard-library` "Library Modules Compile Like Workspace Files";
  - `standard-library` "The UI Intrinsics", which this packet REMOVES.
- `field-visibility` (10b, proposed): private and `package` fields, constructions only where every field is visible (`E4007`).

No other open packet modifies a block this one modifies. Task 1.1 rebases each block on the main specs once those packets are archived.

**Baseline update (task 1.1, on `946ea3a`, "Archive field-visibility").** `drop-and-clone-hooks`, `vararg`, `core-library-in-prism` and `field-visibility` are archived. Every MODIFIED block was compared with the current main text:

- each keeps every scenario of its main requirement, and differs from it only by the changes this packet makes;
- the two blocks written on `core-library-in-prism`'s versions match the archived text: "Library Modules Compile Like Workspace Files" differs only by `prism/composer.prism` in "An imported module is parsed", and the REMOVED "The UI Intrinsics" names the main requirement (whose scenario says `escaping` content for `runApp`, while the library declares it `attached`: the replacing scenario "The public UI declarations" follows the library);
- no ADDED block names a requirement that exists in the main specs, and every REMOVED block names one.

The code names the tasks use all exist: `IntrinsicStruct`, `Type::contains_keeper` (and `prism-borrowck`'s `places::contains_keeper`, which calls it), `checker.rs`'s `reject_composition_in_composable` (the `E4030` list), `is_composition_operation` (in the transform, and on `BuiltinFunction` in `prism-mir`), `uses_composition_operations` (`lib.rs`), `Rewriter::intrinsic`, `adapter_value`, `state_object`, `Rewriter::node`, `root_content_body`, `stability.rs`'s `WeakReference` id, `BodyOwner::RootContent`, `ComposerOperation`, the `RunApp` arm of `lower.rs`, and the three runtime entries. One more user of root content was found: the hot-reload contract's `BodyKind::RootContent` (`crates/prism-codegen/src/contract.rs`), which goes with `BodyOwner::RootContent` (its contract tag 7 is retired). The later packets `layout-primitive` and `layout-in-prism` were already written on this packet's design. `openspec validate compose-library-in-prism --strict` is clean.

**Current state** (main specs and code, with `core-library-in-prism`'s changes assumed):

- `/prism/compose/runtime` declares every name as an intrinsic. `/prism/compose/ui` declares `NodeId`, `HandlerId`, `Node`, `LeafNode` and `runApp` as intrinsics, beside its Prism toolkit code. `/prism/composer` is Prism over 20 externals, on `Int` words.
- **Type checker.** `E4030` comes from a list of `BuiltinFunction`s (`checker.rs`). `IntrinsicStruct` (`crates/prism-typed-tree/src/types.rs`) holds the seven structs with their module and name, markers, the keeper flag (`Composition`) and an equality entry (`HandlerId`).
- **Compose transform** (`crates/prism-compose-transform`):
  - it rewrites composable bodies and any body that calls a composition operation (`uses_composition_operations`);
  - it dispatches the intrinsics (`Rewriter::intrinsic`);
  - it builds one root content adapter body per body that calls `composition`, `setContent` or `runApp` (`BodyOwner::RootContent`, calling the content with the composer and `FORCE`);
  - it lowers `Node`/`LeafNode` to node groups keyed by their ordinal among the region's node calls.
- **Code generation.**
  - Intrinsic structs are one machine word in signatures ("One-Word Intrinsic Structs Travel as Words").
  - `Composition`'s drop glue calls `prism_compose_dispose`.
  - `HandlerId`'s `==` calls `prism_ui_handlers_equal`.
  - `RunApp` calls `prism_ui_run_app` with the root content adapter.
- **Runtime entries.**
  - `prism_compose_start` and `prism_compose_set_content` take word content (`(Int) -> Unit`). The runtime calls it with the composer.
  - `prism_compose_side_effect(composer, effect)`, `prism_compose_current_scope`, `prism_compose_invalidate`, `prism_compose_start_node_group` and `prism_compose_end_group` exist.

## Goals / Non-Goals

**Goals:**

- Every compose declaration that Prism can express without `unsafe`, overloading or lifetimes is written in Prism.
- No compiler path recognizes a library name except by its declaration's catalog entry. Where a property is needed (keeping attached content, running passes, running without a group), it is declared on the declaration.
- One composer-access primitive, in Compose's shape.
- Programs keep their behaviour. Every fixture and example passes, and executable fixtures run under `--check-allocations`. The only accepted differences are listed in D14.
- Constant compile-time cost per declaration and call, and a measured startup.

**Non-Goals:**

- `remember`, `key` and the state core in Prism (D11).
- Annotations in workspace code.
- A typed `Composer`, subcompositions, composition locals.
- Changing any composition semantics: group kinds, keys of composables and regions, skipping, release order.
- Layout in Prism (`layout-in-prism`, item 11).

## Decisions

### D1. What moves to Prism and what stays: D10 challenged (autonomous)

| Declaration | `core-library-in-prism` D10 | Decided here | Why |
|---|---|---|---|
| `Composition`, `composition`, `setContent`, `recompose`, `dump` | Prism | **Prism** (D6) | over externals taking composable content (D4) |
| `RecomposeScope`, `invalidate`, `currentRecomposeScope` | Prism | **Prism** (D7) | `currentRecomposeScope` is an explicit-groups composable |
| `SideEffect` | Prism | **Prism** (D7) | explicit groups, so it never skips |
| `NodeId`, `HandlerId` | Prism | **Prism** (D8) | `package` word fields; `HandlerId.equals` operator |
| `Node`, `LeafNode` | likely stay | **Prism** (D9) | Compose's `ComposeNode`/`ReusableComposeNode` are library functions over `currentComposer`; explicit groups and Compose's node key give the same tree |
| `runApp` | Prism | **Prism** (D10) | over `prism_ui_run_app` taking composable content |
| `remember` | likely stay | **stays** (D11) | keys are overloads; the result is a copy or a borrow of the slot (no lifetimes); permanent identity; state windows |
| `key` | likely stay | **stays** (D11) | Compose's compiler also treats `key` calls itself |
| state core | stays | **stays** (D11) | place rule, guards, versions, cells sized by `T`'s record |
| `WeakReference` | Prism | **stays** (D11) | its `weak()` and `get()` stay intrinsics, and the compiler builds its values for weak captures; as a declared struct it would only lose its one-word representation |

**Compile-time cost.** None by itself.

Rejected:

- **Keeping `Node` and `LeafNode` intrinsic**: Compose does this in library code. With explicit groups (D5), the only cost is a constant node key, which D9 shows keeps node identity.
- **Splitting this change into runtime and toolkit halves**: both halves need the same three pieces (the primitive, annotations, composable content for externals), and the toolkit half is small.

### D2. Library annotations, Kotlin's `@Name`, are the declared markers (autonomous)

The name tests are replaced by properties written on the declarations. Compose expresses such properties as annotations (`@NonRestartableComposable`, `@ReadOnlyComposable`, `@ExplicitGroupsComposable`), and Prism follows Kotlin's syntax.

**Syntax.**

- An annotation is `@` followed by an identifier, with no space between them and no arguments.
- Annotations come before a top-level declaration, ahead of `context(...)`, the visibility and every other modifier. They may be on the declaration's line or on lines of their own.
- `@` is a new token. An `@` anywhere else, or one not followed by an identifier, is `E2001`.

**The three annotations.**

| Annotation | On | Meaning |
|---|---|---|
| `@ExplicitGroupsComposable` | a `compose fun` | starts no group of its own (D5) |
| `@Keeper` | a `struct` | its values keep the attached arguments of the calls that return or change them (D6) |
| `@DrivesComposition` | a non-composable `fun` | calling it in a composable context is `E4030` (D6) |

**Where they may appear.** Annotations are library-only for now: any annotation in a workspace file is `E3002` at its `@`, as `intrinsic` and `external` are. In the library, these are `E4029`:

- an unknown name;
- an annotation on another kind of declaration;
- a repeated annotation;
- `@DrivesComposition` on a composable.

**Compile-time cost.** The parser tests one token at each declaration start. Phase 1 reads at most three flags per declaration, and the checks are a name match per annotation.

Rejected:

- **Contextual modifiers** (`groupless compose fun`, `keeper struct`): Prism's modifiers carry language semantics written in user code (`compose`, `operator`, `escaping`, `attached`). These three are library metadata, which Kotlin and Compose write as annotations, and Compose's own name exists for the first. Three new library-only keywords would also crowd the modifier order.
- **A catalog of declaration paths** (the compiler lists `prism/compose/runtime::Composition` as a keeper): that is a name test moved to a table.
- **Annotations for workspace code now**: each needs its own semantics and review (`@NonRestartableComposable` with hot reload, `@Stable`). Follow-up.

### D3. The composer primitive, first half: `currentComposer()` (autonomous)

`/prism/composer` declares `public intrinsic compose fun currentComposer(): Int`. It gives the composer word of the running body, the same word the protocol functions take.

- **Lowering.** The transform replaces the call by a copy of the body's `$composer` argument: no runtime call.
- **Groups.** It is not a composable call for grouping purposes: it adds no group, and no region needs a group because of it.
- **Compose's surface.** Compose declares `val currentComposer: Composer` as a composable property. Prism has no top-level computed properties, so it is a function, as `currentRecomposeScope()` already is.
- **Placement.** It lives in the protocol module, whose words it returns, not in `/prism/compose/runtime`. Application code never needs it.

**Compile-time cost.** A catalog entry, and one copy per call.

Rejected:

- **A `Composer` struct with a private word**: every protocol function and hand-lowered fixture would change, while the runtime already validates words ("Handles Are Validated").
- **A composable top-level property**: a new declaration form for one name.

### D4. The composer primitive, second half: composable content goes to the runtime, which calls it with the force bit (autonomous)

Externals may take parameters of composable function type (`content: attached compose () -> Unit`), passed by address and moved, as any function value is.

- **New entries.** `prism-compose` gains `prism_compose_start_composable(content) -> u64` and `prism_compose_set_composable(composition, content)`.
- **Calling the content.** The runtime calls such content, when it runs a composition's root, through the function value's descriptor with the composer word and one change word holding only the force bit (bit 0, the transform's `FORCE`). That is the calling convention of `compose () -> Unit` (`code-generation`, composable signatures), and Compose's runtime does the same in `invokeComposable`.
- **`runApp`.** `prism_ui_run_app` takes its content the same way and starts its composition through the new entry.
- **The protocol module.** It keeps word content (`startComposition`, `setCompositionContent`) for hand-lowered code. The runtime records which kind each root holds.
- **What goes away.** The transform's root content adapters (`BodyOwner::RootContent`, `root_content_body`, `adapter_value`) are removed. The transform no longer looks at bodies that are not composable (`uses_composition_operations` goes).

**Compile-time cost.** Less than today: no adapter body per caller, and no scan of non-composable bodies.

**Run-time cost.** None: the runtime makes the call the adapter made.

Rejected:

- **An `invokeComposable(composer, content)` intrinsic used inside a word closure** (`startComposition { k -> invokeComposable(k, content) }`): the closure would capture an `attached` value and hand it to the runtime inside another closure, a path the borrow checker's attached rules (`E5010`) do not cover yet (the C5 gap). It would also keep a closure kind in the transform.
- **Keeping adapters for library callers**: the transform would still recognize `composition` and `runApp`.

### D5. `@ExplicitGroupsComposable`: no group of its own, never skips (autonomous)

**What the transform does.** For a composable with this annotation, the transform emits no body group, no kept parameters, no skipping test and no restart function. The body runs in its caller's group, at the call's position, as the body of Compose's `@ExplicitGroupsComposable` or of an inline composable does. Composable calls in it receive its composer and its change words as usual, and `currentComposer()` gives its caller's composer.

**What the body may contain.** Because the body has no group, a varying group structure inside it would shift its caller's later groups. So the type checker rejects, with `E4029`, in such a body outside closures:

- a composable call inside a branch, a loop, the right operand of `&&`/`||`, the fallback of `?:`, a safe call, or a `when` with composable tests;
- `remember` and `key`.

The body manages any further groups explicitly, through the protocol functions.

**Hot reload.** The annotation is library-only, and library modules never change during a session. So no reload has to find the scopes of a body without a group.

**ABI.** The annotation is not in the ABI: it changes only its own body's lowering. Callers lower its calls as any composable call.

**Compile-time cost.** One syntactic walk of each annotated body, which is library code and linear. In the transform, less work than a restartable body.

Rejected:

- **`@NonRestartableComposable` semantics** (a replaceable group of its own): `Node` and `SideEffect` would add a group per call, changing every dump and costing a group per node per pass.
- **Allowing regions with their own groups inside**: well defined, but no library body needs it. Compose leaves it to the author.

### D6. `Composition` is a `@Keeper` library struct; `@DrivesComposition` gives `E4030` (autonomous)

In `/prism/compose/runtime`:

```kotlin
@Keeper
public struct Composition { private val handle: Int }

operator mut fun Composition.drop() { prism_compose_dispose(handle) }

@DrivesComposition
public fun composition(content: attached compose () -> Unit): Composition =
    Composition(prism_compose_start_composable(content))

@DrivesComposition
public fun Composition.setContent(content: attached compose () -> Unit): Unit =
    prism_compose_set_composable(handle, content)

@DrivesComposition
public fun Composition.recompose(): Bool = prism_compose_recompose(handle)

public fun Composition.dump(): String = prism_compose_dump(handle)
```

**What this keeps.**

- A `Composition` is neither `Copy` nor `Clone`, cannot be constructed outside its file (`field-visibility`), and cannot be taken apart (drop hook, `drop-and-clone-hooks` D5).
- Its drop hook runs wherever the compiler's glue ran: locals, fields, slots and top-level values under the allocation check (`drop-and-clone-hooks` D13).
- Dropping one during a pass is still the runtime's misuse error.

**Keepers.** `@Keeper` sets a flag on the struct's type, which `Type::contains_keeper` reads instead of `IntrinsicStruct::is_keeper`. The keeper rules of `references` ("Weak Captures Keep Their States Borrowed", "Compositions Holding Weak Captures Stay Tracked") are unchanged.

- In the library's own bodies, `content` goes straight to an external's `attached` parameter. A keeper built from an `Int` holds no loans, so the bodies check.
- At call sites, a `Composition` result holds the borrows of the `attached` arguments, and `setContent` writes them through its receiver, as today.

**`E4030`.** It is reported for a direct call of a `@DrivesComposition` function in a composable context, as it was for the four intrinsics. These functions can now be named as values, which the intrinsics could not (`E4013`). A function value of one called during a pass meets the runtime's existing misuse error (`composition operation during a pass`, compose "One Pass at a Time"), which stays the safety net.

**Compile-time cost.** One flag lookup per call (`E4030`) and per local type (keeper), the same as the name tests.

**Run-time cost.** One more call per composition operation (the Prism function, then the external). These run once per pass at most.

Rejected:

- **A keeper derived from the drop hook or an attached parameter**: `Session`-like types with hooks keep nothing, and `runApp` takes attached content but keeps none past its call.
- **Dropping the static `E4030`**: a diagnostic the specs promise.

### D7. `RecomposeScope`, `currentRecomposeScope`, `invalidate` and `SideEffect` in Prism (autonomous)

```kotlin
public struct RecomposeScope : Copy { private val word: Int }

@ExplicitGroupsComposable
public compose fun currentRecomposeScope(): RecomposeScope =
    RecomposeScope(prism_compose_current_scope(currentComposer()))

public fun RecomposeScope.invalidate(): Unit = prism_compose_invalidate(word)

@ExplicitGroupsComposable
public compose fun SideEffect(effect: attached () -> Unit): Unit =
    prism_compose_side_effect(currentComposer(), effect)
```

**Explicit groups.**

- `currentRecomposeScope` must not open a group, or dumps would gain one. A restart group would also be wrong: it would return its own scope.
- `SideEffect` must never skip. Under strong skipping, a restartable `SideEffect` whose effect captures only `Copy` values would skip and drop its effect.

**Equality.** `==` of scopes compares their words structurally, as before.

**Compile-time cost.** None.

**Run-time cost.** One call each, where the transform emitted the runtime call inline.

### D8. `NodeId` and `HandlerId` are library structs over a `package` word (autonomous)

In `/prism/compose/ui`:

```kotlin
package struct NodeId : Copy { package val word: Int }

public struct HandlerId : Copy { package val word: Int }

public operator fun HandlerId.equals(other: &HandlerId): Bool =
    prism_ui_handlers_equal(word, other.word)
```

**Externals.** The UI externals take and return `Int` words: setters take `node: Int`, and registrations return the handler's word, which the toolkit wraps in `HandlerId(...)`. The toolkit modules share the package `prism.compose`, so they read and build the words. No workspace file can, since the package `prism` is reserved (`E1008`).

**Equality.** `HandlerId`'s equality entry becomes its `equals` operator (`operator-conventions` D6), which every equality glue holding one calls: modifiers, kept arguments.

**Compile-time cost.** None.

**Run-time cost.**

- A declared struct is an aggregate, passed by address ("Aggregates Are Passed by Address"), where the intrinsic was a register word. So an `update` closure receives its `NodeId` by address, and comparing handler ids costs a call of `equals` before the external.
- The setters take the word itself, so the calls into `prism-ui` do not change.

Measured by `app_frame_timing` and `ui_run_timing` (D15).

Rejected:

- **`private` words with `package` setter wrappers**: one more call per setter per node per pass, for encapsulation inside the library's own package.
- **Keeping the externals' `NodeId` parameters and reading a pointer in `prism-ui`**: an address per setter call, and a changed runtime ABI for no gain.
- **Small one-word structs in registers for every struct**: signatures would depend on declared layouts, which `code-generation` D10 rejected. It stays a follow-up for Tier 2.

### D9. `Node` and `LeafNode` in Prism, keyed by Compose's node key (autonomous; challenges D10)

```kotlin
@ExplicitGroupsComposable
package compose fun Node(kind: Int, update: (NodeId) -> Unit, content: compose () -> Unit): Unit {
    val composer = currentComposer()
    update(NodeId(prism_compose_start_node_group(composer, 125, kind)))
    content()
    prism_compose_end_group(composer)
}
```

`LeafNode` is the same without `content`.

**The node key.** A node group's key is the constant 125, Compose's `nodeKey`, written in the bodies. Today it is a hash of the parent region and the call's ordinal among the region's node calls.

**Identity is unchanged.** A group reuses the first unvisited old sibling of the same kind and key, so equal keys match in order (compose, "Starting a Group Reuses a Matching Old Sibling"). Among siblings:

- a node call's ordinal is its position among the node groups of its region;
- conditional and repeated code runs in groups of its own;
- the toolkit bodies, the only callers (both functions are `package`), make node calls only at their top level.

So the n-th node group of a region matches the old n-th node group, exactly as the ordinal keys did, and a kind change still makes a new node. Node identity ("Node Identity Survives Unrelated Edits") is kept: a node stays identified by its toolkit composable's group and its position in it.

**Dumps.** Composition dumps show node groups with key 125 instead of a hash, and `dump-mir --composed` of a toolkit body shows a call of `Node` instead of `start_node_group`.

**Compile-time cost.** Less than today: no node-call ordinals.

**Run-time cost.** Per node per pass:

- one more call (the Prism body);
- the `NodeId` built on the stack;
- the node-group entries called through the module's own private externals, as the transform called them.

Measured (D15).

Rejected:

- **Keeping call-site keys by passing the call's key to explicit-groups composables**: a hidden parameter and an ABI flag for no behaviour difference.
- **A key per kind**: equal behaviour, and a mismatched kind already makes a new node.

### D10. `runApp` in Prism (autonomous)

```kotlin
@DrivesComposition
public fun runApp(
    title: &String = "Prism",
    width: Int = 800,
    height: Int = 600,
    content: attached compose () -> Unit,
): Unit = prism_ui_run_app(title, width, height, content)
```

- `prism_ui_run_app` keeps its symbol and takes the composable content (D4).
- `E4030` comes from the annotation.
- `runApp` is not a keeper: its result is `Unit`, so its content keeps borrows only during the call, as today.
- The frame loop, reload points and frame statistics are in `prism-ui` and do not change. `runApp`'s Prism frame stays on the stack for the app's life, and library code is never reloaded.

**Compile-time cost.** None. **Run-time cost.** One call per app.

### D11. What stays compiler-known, and how it is found (autonomous)

**What stays.**

- `remember`. Its keys are up to three values of independent types (Compose has one overload per count, and Prism has no overloading). Its result is a copy of a `Copy` value or a borrow of the slot, valid until the body ends (`owned-state` D6), which no signature states without lifetime annotations. Its result has permanent identity (`strong-skipping` D2), and a calculation that runs opens a state window (`state-handles` D11).
- `key`. Its movable group is keyed by its call site and its data key. As a function it would have one key for every call site, and Compose's compiler treats `key` itself.
- `currentComposer` (D3).
- The state core:
  - `State`, `MutableState`, `mutableStateOf`, `value` (two), `update`, `weak` (two) and `get`;
  - `WeakReference`, since `weak` is a capture function (catalog-listed by `standard-library`), `get` reinterprets a word, and the compiler makes weak handles for captures.

**How they are found.** The catalog keeps them, reached only through their declarations, as `core-library-in-prism` D9 does for the core:

- `IntrinsicStruct` keeps `State`, `MutableState` and `WeakReference`. `IntrinsicStruct::of`, which matches a declaration's module and name against the catalog, is the only place that compares names.
- Every other test reads a catalog property: state object, mutability, conversion target, capture function.
- `stability.rs`'s construction of `WeakReference`'s id goes through the catalog.

**The result.** The compose part of the function catalog has 10 entries:

- `remember`, `key` and `currentComposer`;
- `mutableStateOf`, the two `value` properties, `update`, the two `weak` functions and `get`.

**Compile-time cost.** A shorter catalog, and fewer arms in the transform.

Rejected: **moving the state core over externals**. An external cannot be generic, and a cell is sized and dropped by `T`'s type record: that needs `unsafe` or generic externals, which the user ruled out for now.

### D12. Annotations in the ABI (autonomous)

- `ExportedSymbol::Struct` and `AbiTypeDefinition` gain `keeper`. A file can hold a `Composition` reached through another module's signature without importing `/prism/compose/runtime`, and its borrow checker must still see a keeper.
- `ExportedSymbol::Function` gains `drives_composition`.
- Both are covered by the ABI hash, and `prism abi` prints them (`@Keeper public struct Composition`, `@DrivesComposition public fun composition(...)`).
- `@ExplicitGroupsComposable` is not in the ABI (D5).
- The intrinsic struct entries of `/prism/compose/runtime` are now `State`, `MutableState` and `WeakReference` only.

**Compile-time cost.** Two flags per exported symbol in Phase 1.

### D13. Typed tree, MIR and code generation (autonomous)

**Removed, with their arms in the validators, dumps, codec and backend:**

- `BuiltinFunction::{Composition, SetContent, Recompose, DumpComposition, InvalidateScope, RunApp, SideEffect, CurrentRecomposeScope, UiNode, UiLeafNode}`;
- `ComposerOperation::{StartComposition, SetContent, Recompose, Dump, Invalidate, SideEffect, CurrentScope, StartNodeGroup}`;
- `BodyOwner::RootContent`;
- `IntrinsicStruct::{Composition, RecomposeScope, NodeId, HandlerId}`.

Their codec tags are retired, never reused. Every cache entry is keyed by the compiler identity, so `FORMAT_VERSION` stays.

**Added:** `BuiltinFunction::CurrentComposer`, which the transform lowers to a copy of `$composer`.

**Code generation.**

- Calls of the moved declarations are ordinary `Callee::Declaration` calls.
- Glue loses `Composition`'s dispose special case (the hook does it) and the equality entry.
- Signatures pass only `State`, `MutableState` and `WeakReference` as words.
- `prism dump-layout` still shows `Composition` and `RecomposeScope` as 8 bytes, a struct of one `Int`.

**Compile-time cost.** None. Fewer cases throughout.

### D14. Behaviour unchanged, with listed differences (autonomous)

Every program keeps its output and exit status, under `--check-allocations` included, and every example and session runs as before. The accepted differences:

- **Dumps.**
  - `dump-mir --composed` shows ordinary calls of `composition`, `runApp`, `SideEffect`, `currentRecomposeScope`, `Node` and `LeafNode`, and no `#root-content` closures.
  - Composition dumps show node group keys of 125.
  - The typed tree and MIR name the declarations (`call prism/compose/runtime::composition`).
- **`prism abi`** of `/prism/compose/runtime` and `/prism/compose/ui`: Prism signatures with annotations, the structs with their fields.
- **`--explain-cache`** lists `prism/composer.prism` for a program importing `/prism/compose/runtime` or a UI module, since both now import it.
- **Shadowing.** A workspace extension named like a moved member (`fun Composition.recompose(): Bool`) shadows the import instead of being `E4010`, as for `Int.dp` in `core-library-in-prism` D7.
- **Forgery.** Constructing or taking apart `Composition` or `RecomposeScope` in a workspace file is `E4007` (hidden field) instead of "no constructor and no fields".

### D15. Compile speed and startup (autonomous)

**Startup**, measured on an idle machine before the first implementation commit and after the last, and recorded here:

- `run_timing` cold and warm (target 15 ms);
- `warm_run_timing`'s stages;
- `ui_run_timing`;
- `app_frame_timing` (the paced toggle and typing frames of `strong-skipping`, which this change touches through `Node`, `NodeId` and handler equality);
- `list_timing`.

**Expected effect.**

- Cold runs compile about 15 more small Prism functions once per cache. A program importing `/prism/compose/runtime` or a UI module also parses and checks `/prism/composer` (142 lines). Warm runs load them cached.
- Frames gain one call per node and per handler comparison, the costs of D8 and D9.

A regression of more than 5 % in a cold, warm or frame median is investigated before archiving. One remedy, if a node's extra call shows: declare the hot externals in the calling module, which D9 already does for node groups.

**Compile timing.**

- No new stage. The annotation checks are per declaration and library-only, so workspace shapes cannot use them.
- `compile_timing`'s `many-composables`, `composable-calls`, `ui-calls` and `nested-compose-ui` shapes are re-run under `PRISM_ASSERT_TIMING=1`. They must stay within ×4.6 per ×4 and should not slow down, since the transform does less.

**Baseline (task 1.2)**, on `946ea3a` before any code change, release build, idle machine (2026-10-20):

| Measurement | Median | p90 / p99 |
|---|---|---|
| `run_timing` cold, total to main / wall | 11.07 / 14.11 ms | p90 12.30 / 15.67 ms |
| `run_timing` warm, total to main / wall | 2.10 / 4.87 ms | p90 2.14 / 4.95 ms |
| `warm_run_timing` `many_files` 1024, cold / warm front end | 219.8 / 79.2 ms | |
| `ui_run_timing`, a click among 1000 texts | 0.222 ms | |
| `app_frame_timing` counter click frame | 0.003 ms | p99 0.004 ms |
| `app_frame_timing` todo toggle recomposition / frame | 0.142 / 1.876 ms | p99 0.362 / 3.424 ms |
| `app_frame_timing` todo typing recomposition / frame | 1.499 / 2.408 ms | p99 3.097 / 4.555 ms (the 4 ms p99 target was already missed) |
| `app_frame_timing` todo idle click recomposition | 4.541 ms (once) | frames that change nothing allocate nothing: met |
| `app_frame_timing` todo first frame, cold / warm | 31.1 / 3.06 ms | |
| `list_timing`, a million adds | 7.55 ms | p90 9.48 ms |

`compile_timing` (`PRISM_ASSERT_TIMING=1`, passed): `many-composables` 256/1024/4096 lower 9.2/40.8/167.3 ms, compose 2.1/9.5/39.4 ms; `ui-calls` compose 3.4/16.3/75.3 ms, borrow 11.5/51.7/226.5 ms; `composable-calls` 4096 compose 2.9 ms; `nested-compose-ui` 128 compose 16.6 ms, lower 50.0 ms (all stages ×1.9 to ×2.05 per ×2).

### D16. No new error code (autonomous)

- **`E2001`**: a malformed annotation (`@` alone, `@` before a non-identifier, an annotation inside a body).
- **`E3002`**: any annotation in a workspace file.
- **`E4029`**:
  - an unknown annotation, one on the wrong kind of declaration, a repeated one, and `@DrivesComposition` on a composable;
  - an explicit-groups body breaking D5's rules;
  - a library declaration of a removed intrinsic.
- **`E4030`**: a call of a `@DrivesComposition` function in a composable context, which names the function.

## Risks / Trade-offs

- **[Run time of toolkit frames]** `NodeId` by address, one more call per node, and handler comparisons through `equals` (D8, D9). → Measured against the paced targets (D15). Tier 2 inlines all three.
- **[Function values of `@DrivesComposition` functions]** They escape the static `E4030`. → The runtime's misuse error, as for the protocol module's functions today.
- **[Node identity under a constant key]** → D9's argument. Fixtures "Node Identity Survives Unrelated Edits", "Nodes Follow Their Composables" and the example sessions check it. The reviewer should probe a toolkit body with two node calls and a kind change.
- **[The runtime knows the force bit]** `compose-runtime` D1 rejected "a force word passed by the runtime (it would fix the change-word layout here)". → Only the root call of composable content uses it, and it is bit 0 of the first word, which the code-generation spec already fixes for every composable signature. The protocol's word content and restart functions are unchanged.
- **[Fixture churn]** Dumps, ABI outputs and explanations (D14). → Task 6.3 re-blesses and reads each diff against D14's list.
- **[Packets landing in between]** → Task 1.1 rebases.

## Migration Plan

Programs keep their meaning, apart from D14's shadowing and forgery errors. The library sources change as follows:

- `composer.prism` gains `currentComposer`.
- `compose/runtime.prism` writes D6 and D7 over private externals.
- `compose/ui.prism` writes D8 to D10, and imports `/prism/composer`.
- `foundation.prism`, `layout.prism` and `material3.prism` pass `node.word` to setters and wrap registrations in `HandlerId(...)`.

Runtime crates gain two entries and change `prism_ui_run_app`'s content. The build cache is keyed by the compiler identity, so nothing cached is reused. Rollback: revert the commits.

## Open Questions

- Annotations for workspace code, starting with `@NonRestartableComposable` and how hot reload finds the scopes of a body without a group.
- `remember` in Prism once overloading exists, and whether its result typing can then be stated with an elision rule over `currentComposer`.
- A typed `Composer` for the protocol module.

## Implementation Notes

- **The annotation is one token** (group 2). D2 had `@` as a token of its own followed by an identifier. The formatter's corpus tests insert comments and spaces between any two tokens and expect the parse to survive, which an `@` and its name as two tokens would not. The lexer reads `@` directly followed by a name as one `Annotation(name)` token, and an `@` without a name as a lexer error, reported as `E2001` like every lexer error. The grammar is unchanged: nothing can come between `@` and the name.
- **Keepers are read through the type table** (group 2). `Type::contains_keeper` takes the `TypeTable`, so the borrow checker's `carries_loans`, `contains_keeper` and `writes_loans_through` take it too; `BorrowSet` computes the returned locals once (they need the table). Until group 5 annotates `Composition`, `contains_keeper` also accepts the catalog's `IntrinsicStruct::is_keeper`.
- **`E4030` from the flag** (group 2). The checker keeps the declaration paths of the `@DrivesComposition` functions it declares or imports, plain and extensions, and reports a direct call of one in a composable context; the `BuiltinFunction` list stays until groups 5 and 6 remove the intrinsics.
- **`runApp`'s content is composable content from group 3** (group 3). `prism_ui_run_app` calls its content with the force bit as soon as the runtime entries exist, so the transform stopped wrapping `runApp`'s content in an adapter in the same commit (the `RunApp` built-in still exists until group 6); `mir/the-adapter-of-an-app`, the fixture of the removed requirement "runApp Gets a Root Content Adapter", was deleted with it. Root content kinds are a `RootContent` enum of `prism-compose`, recorded per composition (`composable_root`), and `FunctionValue::call_composable` makes the call.
- **"Protocol calls inside a composable" corrected** (group 3). The scenario's `composition { Cell(1) }` then `c.setContent { Cell(2) }` gives two different content lambdas, whose groups are keyed by the call they are arguments of, so `Cell`'s group is new on the second pass and its slot computed again. The scenario now recomposes the same content through a state: `val n = mutableStateOf(1); val c = composition { Cell(n.value) }; n.value = 2; c.recompose()`.
- **No CLIF snapshot for library cases** (group 3). Library fixture cases cannot dump CLIF, so "Content given to an external" is a test of `crates/prism-codegen/src/lowering_tests.rs` (`content_given_to_an_external`), which lowers a library module through the front end and reads the dump.
- **Explicit groups through the trees** (group 4). The annotation reaches the transform as `FunctionItem::explicit_groups` and `Body::explicit_groups` (encoded by the MIR codec); MIR dumps write such a body `compose explicit_groups fun` (`compose composed explicit_groups fun` once composed). The type checker's walk (`explicit_groups_violations`) treats the subject of a `when` and the condition of an `if` as ungrouped, and a `when`'s patterns, guards and branches, a loop statement, a safe call, the right operand of `&&`/`||` and the fallback of `?:` as grouped, as the MIR builder's regions are.
