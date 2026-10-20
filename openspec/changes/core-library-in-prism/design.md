# Design: core-library-in-prism

## Context

See `proposal.md` for the motivation. This change is the first half of item 10 of "After Milestone 2: the review round" in `ROADMAP.md` (`library-in-prism`). That item is the core of agreed item **A4** of `docs/milestone-2-review-notes.md`:

- "as much as possible ... normal prism code that is imported like any other code" (user, 2026-10-20);
- a minimal core of compiler features (scalars, `T?`, `&T`, function types, string literals, `compose fun` and its transform, `external fun`, one composer-access primitive);
- no `unsafe` for now, so `List`, `String` and `Heap` stay Rust-backed (user decision);
- explicit imports and no prelude (done by `explicit-library`).

The user is away. Every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Decisions the user took in the review are marked **(user direction 2026-10-20)**. Compile speed is a primary criterion (AGENTS.md §5), so each decision states its compile-time cost.

**Baseline.** Proposed on top of `354e609` ("Implement operator-conventions: reconcile"). The main specs are those after `owned-state`, `strong-skipping` and `explicit-library` (archived). Three packets land before this one:

- `operator-conventions` (L1, being implemented): the `operator` modifier, operator calls, `List<T>.contains` as an `intrinsic operator`.
- `drop-and-clone-hooks` (L2, proposed): drop and clone hooks, which `compose-library-in-prism` needs; this change uses none.
- `vararg` (L4, proposed): `listOf` written in Prism, and `print`, `println`, `remember` and `key` left to this item (its D11).

Several of this packet's blocks are written on another packet's version and must be rebased on the main specs once those archive (task 1.1):

- `standard-library` "Intrinsic Operators" (ADDED by `operator-conventions`). `openspec validate --strict` reports, as INFO, that the header is not yet in the main spec.
- `standard-library` "The Collections Module Declares Lists and Their Helpers" and "Intrinsic Calls Are Typed by Their Signatures" (`vararg`'s MODIFIED versions).
- `mir` "List Operations Are Built-In Calls" (`vararg`'s version).
- `typed-tree` "List Operations Are Identified", written for the state after `vararg`, whose delta leaves that requirement's `listOf` sentence stale.

No other open packet modifies a block this one modifies.

**Baseline update (task 1.1, 2026-10-20).** Re-based on `6600fa53` ("Archive vararg"): `operator-conventions`, `drop-and-clone-hooks` and `vararg` are archived. Every MODIFIED block was compared with the current main text:

- The blocks written on other packets' versions match main now, apart from this change's own edits: `standard-library` "Intrinsic Operators" (now in main, so MODIFIED is right and the INFO line is gone), "The Collections Module Declares Lists and Their Helpers" and "Intrinsic Calls Are Typed by Their Signatures", `mir` "List Operations Are Built-In Calls", and `typed-tree` "List Operations Are Identified" (whose main text still calls `listOf` a built-in construction; this block corrects it to `prism/collections::listOf`).
- No MODIFIED block drops a scenario of its main requirement, and no ADDED requirement exists in main.
- Two main requirements would have gone stale and are added as MODIFIED blocks: `collections` "Lists Answer Membership Tests" (`contains` is declared by `/prism/core`), and `standard-library` "Intrinsic Declarations", whose scenario "An intrinsic with another signature is rejected" used `Int.toString(radix: Int)`, an intrinsic this change removes; it now uses `List<T>.removeLast(count: Int)`, and its fixture follows.
- Every function and file the tasks name exists under that name, and the retired codec tags (5, 10, 11, 12, 38 and 39) are those of `crates/prism-mir/src/codec.rs`.

`openspec validate core-library-in-prism --strict` is clean with no INFO line.

### The inventory

Every `intrinsic` and `external` declaration under `library/prism/`, and every compiler special case for a library name, classified as one of:

- **Prism**: written in Prism by this change.
- **compose**: decided by `compose-library-in-prism` (D10).
- **stays**: stays in the compiler or the runtime, with the reason in D2.
- **done**: already Prism.

| Module | Declarations | Today | Class |
|---|---|---|---|
| `/prism/core` | `print`, `println` | rule-typed intrinsics | stays (D8) |
| | `toString()` of `Int`, `Double`, `Bool`, `Char` | intrinsics, plus a type-checker path by name | **Prism** (D4) |
| | `String.length` | intrinsic property, an inline load | stays (D4) |
| `/prism/collections` | `listOf` | rule-typed intrinsic | done by `vararg` |
| | `size` | intrinsic property, an inline load | stays (D4) |
| | `indices` | intrinsic property | **Prism** (D4) |
| | `isEmpty`, `isNotEmpty` | intrinsics | **Prism** (D4) |
| | `contains`, `indexOf` | rule-typed intrinsics (`==` on `T`) | stays (D4) |
| | `add`, `insert`, `removeAt`, `removeLast`, `reverse`, `clear` | intrinsics | stays (D4) |
| | the 13 helpers | Prism | done |
| `/prism/system` | `exitProcess` over `prism_rt_exit` | Prism over an external | done |
| `/prism/composer` | 18 wrappers over 20 externals | Prism over externals | done |
| `/prism/compose/runtime` | `Composition`, `RecomposeScope`, `State`, `MutableState`, `WeakReference` (intrinsic structs); `composition`, `setContent`, `recompose`, `dump`, `remember`, `key`, `currentRecomposeScope`, `invalidate`, `mutableStateOf`, `value` (two), `update`, `weak` (two), `get`, `SideEffect` | intrinsics | compose (D10) |
| `/prism/compose/ui` | `NodeId`, `HandlerId`, `Node`, `LeafNode`, `runApp` | intrinsics | compose (D10) |
| | `Int.dp`, `Int.sp` | intrinsic properties, lowered as constructions | **Prism** (D7) |
| | 12 `package` externals (setters, handler registration, chain overflow) | externals of `prism-ui` | stays (native UI runtime) |
| `/prism/compose/{foundation,layout,material3}` | none | Prism over `ui`'s `package` externals | done |

Compiler built-ins with no declaration, all **stays** (D2):

- the built-in types and `Range`;
- `Heap(...)`, `unbox()` and `clone()`;
- a string's character `s[i]`;
- the operators of the built-in types;
- structural `==`, templates, `..` and `..<`;
- `for` over `Range` and `List`.

Special cases by name found by grepping the type checker, MIR lowering, the compose transform and the code generator:

- **Removed by this change.** The type checker's scalar `toString` path (`member == TO_STRING && has_builtin_to_string(...)`, then `infer_to_string_call`), and the `LIST_SIZE`/`indices` branch of list property reads.
- **Stays a convention.** The template and `print` rule that calls `toString()` by that name, as `operator-conventions` calls `plus`.
- **compose.** The tests of `IntrinsicStruct` values, by module and name:
  - `is_state_handle`, behind the `value` rule (`checker/state.rs`'s `VALUE`), state places and guards;
  - `is_keeper` (`Composition`);
  - `equality_entry` (`HandlerId`);
  - `is_composition_operation` and the transform's `intrinsic` dispatch on `Remember`, `Key`, `SideEffect`, `CurrentRecomposeScope`, `Composition`, `SetContent`, `RunApp`, `UiNode`, `UiLeafNode`, `Recompose`, `DumpComposition` and `InvalidateScope`.

## Goals / Non-Goals

**Goals:**

- Every declaration of the core, collections and unit-property layer that Prism can express without `unsafe`, traits or overloading is written in Prism.
- Extension properties as a language feature in every file.
- A rule for members of the built-in types that admits members written in Prism.
- A recorded classification of the whole library, so that `compose-library-in-prism` starts from it.
- Programs keep their behaviour. Every existing fixture and example passes, executable fixtures run under `--check-allocations`, and the only accepted differences are dumps naming declarations instead of built-ins, `E4010` messages naming `/prism/core`, and the unit-property shadowing of D7.
- Constant compile-time cost per use, and a measured startup.

**Non-Goals:**

- Anything in `/prism/compose/runtime` and the toolkit's intrinsics: `compose-library-in-prism`.
- `var` extension properties with setters, `compose` or context-parameter extension properties, and extension properties whose type is inferred.
- `String.length`, `List.size`, the mutating list members, `contains`/`indexOf`, `Range` or `Heap` as Prism. Each needs `unsafe`, traits, or inlining that Tier 1 lacks (D2, D4).
- Changing `print`/`println`'s surface (D8).
- Overloading, traits, `Any`.

## Decisions

### D1. Split `library-in-prism` into `core-library-in-prism` and `compose-library-in-prism` (autonomous)

The inventory above shows two halves of very different size.

**This change.** The core, collections and unit-property layer needs one language feature (extension properties, D5) and one rule (members written in Prism, D3). Everything else in it either moves with existing features or stays for reasons that need no design (D2).

**`compose-library-in-prism`.** The compose runtime and the toolkit handles need at least four decisions the user may want to review before code:

- field visibility (or constructor visibility), so a library struct wrapping a runtime handle cannot be forged. That is the reason `compose-transform` D16 made `Composition` intrinsic: "Rejected: forgeable library structs";
- the composer primitive, in both directions: `currentComposer`, and running a composable for a composer word, which today is the root content adapter of `composition`, `setContent` and `runApp`;
- declaration markers for composables that run in their caller's group or never skip, as Compose's `@ReadOnlyComposable` and `@NonRestartableComposable` do for `currentRecomposeScope` and `SideEffect`;
- declared replacements for the name tests on `Composition` (keeper, `E4030`) and on the state types (state places, guards, versions, identity equality).

That change also rewrites `Composition` with a drop hook (L2), which its own reviewer must probe.

So this change goes first, and `ROADMAP.md`'s item 10 is split into two sub-items. Each half gets its own propose, implement and review loop (`docs/agent-workflow.md` §1), and `layout-in-prism` (item 11) comes after both.

**Compile-time cost.** None.

Rejected:

- **One change**: about four design decisions and a dozen compiler subsystems at once, with one reviewer for all of them.
- **Splitting the compose half now into runtime and state**: that is `compose-library-in-prism`'s proposer's call once field visibility and the primitive are designed. D10 records the classification it starts from.
- **Proposing the compose half first**: it would build on member and property rules this change settles.

### D2. What stays in the compiler, and why (user direction 2026-10-20 for the goal and for no `unsafe`; the criteria autonomous)

A declaration stays an intrinsic, or a built-in without a declaration, only for one of three reasons:

1. **It reads or changes the representation of a Rust-backed built-in type** (`String`, `List`, `Heap`). Prism code cannot do that without `unsafe`, which the user ruled out for now. Externals cannot stand in for the generic ones: an external is not generic (`standard-library`, "External Declarations Are Restricted"), and a list operation's machine code depends on the element's size and drop glue.
2. **Its typing needs a feature Prism lacks.** `print(x)` takes any type with a text form, Kotlin's `Any?` (D8). `xs.contains(x)` needs `==` on `T`, a trait.
3. **It is syntax or an operator of a built-in type**, which `operator-conventions` D3 keeps built in, ahead of operator functions: `+` on `Int`, `s[i]`, `==`, templates, `..`. So `Range` stays built in as well: it is what `..` builds, and `for` over a range is a counted loop that Tier 1 would otherwise run as three calls per iteration.

Everything else is Prism, over `external` runtime entries where it needs the runtime.

**Compile-time cost.** None by itself.

Rejected:

- **Moving operators of built-in types to library `operator fun`s** (Kotlin's `Int.plus` is an intrinsic member too): every arithmetic expression would cost a lookup and a call in Tier 1.
- **`Range` as a library struct with the cursor protocol**: three calls per iteration until Tier 2 inlines.

### D3. `/prism/core` is the only member module and declares every member of the built-in types, intrinsic or written in Prism (autonomous)

**Today.** The members of a built-in type are the *intrinsic* extensions of the member modules `/prism/core` and `/prism/collections` (`explicit-library` D2). A member written in Prism would be an ordinary extension: not in scope without an import, and shadowable.

**The obvious rule fails.** "Every extension a member module declares on a built-in type is a member" would make `map`, `filter` and the other helpers members too, against `explicit-library` D2 ("helpers as members" rejected).

**The rule chosen.**

- The members of the built-in types move to one module, `/prism/core`, which already holds the scalars' and `String`'s.
- Every extension, function or property, that `/prism/core` declares on a built-in type is a member, intrinsic or written in Prism.
- `/prism/core` declares nothing else but `print` and `println`, which are not extensions, so the rule cannot catch anything else.
- `/prism/collections` keeps `listOf` and the helpers and becomes an ordinary module, imported as today. It imports `core` for the members its bodies use (library files see nothing implicitly, `explicit-library` D7).
- `prism-library`'s `MEMBER_MODULES` becomes `["prism/core"]`.
- The member scope, built once per program from that module's ABI (`members.rs`), takes every extension on a built-in receiver. A Prism member is an `Extension` with no built-in and a declaration path, called as any extension.

**Visible effects.**

- A program that imports nothing no longer parses or checks `/prism/collections`: 170 lines today.
- Check keys of workspace files append one member-module hash instead of two (`build-cache`).
- `E4010` messages for a workspace extension named like a list member name `/prism/core`.
- Users write the same imports as before: `import /prism/collections` for `listOf` and helpers, `import /prism/core` for `println`.

**Compile-time cost.**

- Member lookups are unchanged: one hash lookup in the shared `MemberScope`.
- A program without imports parses and checks one library module instead of two.
- A program importing `/prism/collections` parses the same lines as before.

Rejected:

- **A `member` modifier on extensions**: a new declaration form for one library's needs.
- **Keeping two member modules with "only intrinsic extensions are members"**: no member could be written in Prism.
- **A new `/prism/builtins` module for the members**: a third module name for what `/prism/core` already is.
- **"Extensions declared in their receiver type's file are members"** for every type: a semantic change for all user code. It is perhaps worth discussing once types have bodies or traits.

### D4. Which members are written in Prism (autonomous)

Written in Prism in `/prism/core`:

- `public fun Int.toString(): String = prism_rt_int_to_string(this)`, and the same for `Double`, `Bool` and `Char`, over the runtime's existing entries. Each is declared as a private external whose machine signature (a result address, then the scalar) already matches the runtime table, as the `standard-library` scenario "A string result through an address" shows. The type checker's dedicated path (`infer_to_string_call`, `has_builtin_to_string`) goes, and `x.toString()`, `x?.toString()` and `h.toString()` on `Heap<Int>` take the ordinary member path, which already handles `?.` and the `&`, `&mut` and `Heap<T>` layers.
- `public fun <T> List<T>.isEmpty(): Bool = size == 0` and `isNotEmpty()` (`size != 0`).
- `public val <T> List<T>.indices: Range get() = 0..<size` (D5).

Kept intrinsic:

- **`String.length` and `List.size`**: an inline load of the stored count (`code-generation` D7 of `runtime-strings-and-drops`). Over an external, every read would become two calls, the slot call and the external, in Tier 1, the hottest members of both types.
- **`add`, `insert`, `removeAt`, `removeLast`, `reverse` and `clear`**: they move elements of a Rust-backed buffer of any element type (D2, reason 1).
- **`contains` and `indexOf`**: they need `==` on `T` (D2, reason 2).

Rejected:

- **`removeLast` as `removeAt(size - 1)`**: the runtime error of an empty list would change its message and be located in `/prism/core`.
- **`clear` as a loop of `removeLast`**: elements would be dropped last to first, an order hooks (L2) would show.
- **`String.length` over an external**: as above.

**Compile-time cost.**

- `toString` on a scalar: one member lookup, the same as today's path.
- `isEmpty` and `indices` on a `List<E>`: one tiny instance per element type and program (`isEmpty<Int>`), planned from summaries and cached like any instance (`compilation-and-abi`, "Member Modules Are Inputs of Every Workspace File", which gains the scenario "Instances of generic members").

**Run-time cost.** One call through the function table per use (plus the external's call for `toString`), where the built-ins were inline. Tier 2 inlines them.

### D5. Read-only extension properties with a getter, Kotlin's form, in every file (autonomous)

`[visibility] val [<T, ...>] Receiver.name: Type get() = expression`, or `get() { block }`, with the getter on the same line or the next, as Kotlin writes:

```kotlin
val Int.dp: Dp get() = Dp(this)
val <T> List<T>.indices: Range
    get() = 0..<size
```

Rules:

- `get` is a keyword only after an extension property's type, so no program loses a name.
- The type is always written: no inference, so Phase 1 needs nothing new and a private property costs no inference key.
- Type parameters must occur in the receiver type (Kotlin's rule), so a read solves them from the receiver alone (`E2001` otherwise, checked syntactically).
- An initializer is `E2001`, with help suggesting `get() =`, since an extension property stores nothing (Kotlin rejects it too).
- `var` and setters are deferred: no library need, and `MutableState.value` is the compiler's.
- `compose`, receiver modes and context parameters are `E2001`.
- `intrinsic val` keeps its form, with no getter.

This closes the `standard-library` follow-up "Extension properties outside the library (`val String.twice: Int get() = ...`)".

**Compile-time cost.** The parser tests one token after the type. Checking a getter is checking a function body.

Rejected:

- **Keeping `intrinsic val` as the only property form**: `dp`, `sp` and `indices` would stay intrinsics.
- **Unit functions (`16.dp()`)**: not Compose's surface (`ui-toolkit` D7, user direction to follow Compose).
- **An initializer form (`val Int.dp: Dp = Dp(this)`)**: it reads as a value computed once.
- **Library-only extension properties**: no reason to deny user code a Kotlin feature that costs nothing.

### D6. An extension property is an extension function read without parentheses (autonomous)

**Resolution.**

- A read `x.name` with no field `name` looks up a member property, then the file's own extension properties, then its imports' (`E4012` when several imports match, `E4006` when none does).
- Properties share the extension table with functions: one entry per receiver type constructor, name and context. So a property and a function of one name on one receiver in one file are `E4010`, as two extensions are.
- A property named like a field or a member is `E4010`, as an extension of that name is.
- `x.name()` calls the value read, which is `E4004` for a non-function, as for `size` today.
- An assignment is `E4003`, with a message saying the property has no setter.

**The getter.**

- A getter is a function `<module>::<Receiver>.<name>` with a shared receiver and no parameters, checked as an extension body.
- `this`, implicit `this` and the elision rules work as in such a body. So a getter returning `&T` ties the borrow to the receiver (rule 2), and a getter is not a composable context.
- A read is a call of the getter with a shared receiver borrow. In the typed tree, MIR, the borrow checker, code generation and hot reload it is an ordinary extension call: nothing downstream learns properties.

**Compile-time cost.** A failed field test, then one member lookup and at most two extension-table lookups per read. Shape `extension-properties` (D14).

Rejected:

- **A separate namespace for properties** (a property and a function of one name coexisting, as Kotlin allows): two lookups per read and per call for a confusing surface.
- **A dedicated property node in the typed tree**: every later stage would learn it, for what is a call.

### D7. `Int.dp` and `Int.sp` become ordinary Prism extension properties of `/prism/compose/ui` (autonomous)

`public val Int.dp: Dp get() = Dp(this)` and `public val Int.sp: TextUnit get() = TextUnit(this)`. Two consequences:

- **No longer members.** Like Compose's `dp`, and like `explicit-library` D2 described them ("the helpers, `Int.dp` and `Int.sp` are extensions"), they are ordinary imported extensions, not members of `Int`. A file's own `Int.dp` shadows the import instead of being `E4010`. The `standard-library` requirement "The Unit Properties" is removed, and "Unit Properties Are Written in Prism" replaces it.
- **The construction moves into the getter.** `BuiltinFunction::{DpOfInt, TextUnitOfInt}` and MIR's construction special case (`mir`, "Unit Properties Lower to Constructions", D7 of `ui-toolkit`) go, and a read is a call.

**Compile-time cost.** None measurable.

**Run-time cost.** One call per unit read, where an aggregate was built inline. A composable body using units builds a few per pass. The frame and recomposition targets of `app_frame_timing` and `ui_run_timing` are re-measured (task 7.2).

Rejected:

- **Keeping them intrinsic for speed**: a special case for a two-word constructor, which Tier 2 inlines.
- **Keeping member status for Prism-written `dp`**: there is no rule for it outside `/prism/core` (D3), and Compose's `dp` is shadowable.

### D8. `print` and `println` stay rule-typed intrinsics (autonomous; settles `vararg` D11 for them)

Kotlin's `println(message: Any?)` takes one value of any type, and Prism has no `Any`, no traits and no overloading. Their rule ("Output Built-In Arguments": the argument's text form, as a template part's) cannot be a signature. They stay intrinsics of `/prism/core`, lowered as today: scalars and `String` straight to the runtime's print entries, other types through their `toString()`.

**Compile-time cost.** None.

Rejected:

- **`println(message: &String = "")` in Prism**: `println(1)` would stop compiling, unlike Kotlin and every example.
- **A Prism `print(text: &String)` that the rule calls with the argument's template** (typing stays a rule, lowering becomes Prism): an allocation per printed scalar, and two declarations for one function, for no gain in the catalog.
- **`print(vararg values: T)`**: homogeneous, so still one type per call (`vararg` D11).

### D9. The resulting catalog, and how intrinsics are found (autonomous)

After this change, the core part of the intrinsic catalog (`crates/prism-typechecker/src/intrinsics.rs`) holds 12 entries, all of module `prism/core`:

- `print`, `println` (rule-typed);
- `String.length`;
- `List<T>.size`, `contains` (an `intrinsic operator`, and rule-typed, as is `indexOf`), `indexOf`, `add`, `insert`, `removeAt`, `removeLast`, `reverse`, `clear`.

It loses the four `toString` entries, `isEmpty`, `isNotEmpty`, `indices`, `Int.dp` and `Int.sp`, with their `BuiltinFunction` variants. `listOf` is already gone by `vararg`. The compose entries (`/prism/compose/runtime`, `/prism/compose/ui`) are unchanged here.

As before, an intrinsic is reached only through its declaration: a use resolves to a declaration by ordinary lookup, and the declaration's catalog entry, keyed by module path, receiver and name and checked against its signature (`E4029`), gives the built-in. After this change no type-checker path tests a member's spelling to decide that a call is built in, except:

- the built-in members of every type (`clone()`, `unbox()`);
- the conventions (operator names, `toString()` for text forms);
- the compose names, which D10 hands over.

**Compile-time cost.** A shorter catalog: its linear lookup per intrinsic declaration and ingested symbol shrinks.

### D10. The compose half: the classification `compose-library-in-prism` starts from (autonomous; recorded here, decided there)

| Declaration | Recommendation | What it needs |
|---|---|---|
| `Composition` | library struct over a composition handle, with a drop hook that calls `disposeComposition` (L2) | field visibility; the keeper property declared, not tested by name |
| `composition(content)`, `Composition.setContent` | Prism over `/prism/composer`'s `startComposition` and `setCompositionContent` | the composer primitive's second half: a composable run with a given composer word, Compose's `invokeComposable` (today the transform's root content adapter); `E4030` from a declaration marker |
| `recompose`, `dump` | Prism over `recomposeComposition` and `dumpComposition` | `E4030` marker for `recompose` |
| `RecomposeScope`, `invalidate` | library `Copy` struct over a scope word | field visibility |
| `currentRecomposeScope()` | Prism over `currentComposer` and `currentScope` | a composable that runs in its caller's group (`@ReadOnlyComposable`); otherwise it would return its own scope |
| `SideEffect` | Prism over `currentComposer` and a runtime entry | a composable that never skips (`@NonRestartableComposable`): under strong skipping a restartable `SideEffect` would skip and drop its effect |
| `remember` | likely stays intrinsic | its result is a copy for a `Copy` `T` and otherwise a borrow of the slot, valid until the body ends (`owned-state` D6). No signature states that without lifetime annotations, and the transform gives it call-site groups and permanent identity (`strong-skipping` D2). Its slot code is already lowered to protocol calls |
| `key` | likely stays intrinsic | a movable group keyed by its call site and data key. Written in Prism, its group key would be the same for every call site, so state could move between two `key` calls with equal data keys. Compose's compiler also treats `key` calls itself |
| `State`, `MutableState`, `mutableStateOf`, `value`, `update` | stay compiler-known | the `value` place rule, flags and guards, versions, identity equality, the `MutableState` to `State` conversion and `E4031`/`E4032` are compiler features of these types. A cell is sized and dropped by `T`'s type record, which no external can take. Replace the `IntrinsicStruct` name tests by catalog data of the declarations (for example a state flag on the struct entry) rather than names |
| `weak()`, `WeakReference` | Prism with field visibility (a word naming the cell) | the generic drop of L2 is not needed (a weak reference is `Copy`) |
| `WeakReference.get()` | stays | it views the reference's own word as a state object, a reinterpretation that needs `unsafe` |
| `NodeId`, `HandlerId` | library `Copy` structs over words | field visibility; `HandlerId`'s runtime equality becomes `operator fun HandlerId.equals` over `prism_ui_handlers_equal` (L1) |
| `Node`, `LeafNode` | likely stay intrinsic | node groups keyed by their call site through the applier. As Prism functions each node would also get its own body group (a group per node, changed dumps) unless composables that run in their caller's group exist |
| `runApp` | Prism over `prism_ui_run_app` | the composer primitive's second half and the `E4030` marker |
| UI setters and registration | stay externals | native UI runtime |

**Compile-time cost.** None here. Each item's cost is stated in that change.

### D11. Typed tree, MIR and code generation: moved members are ordinary calls (autonomous)

- `BuiltinFunction::{ToString, ListIsEmpty, ListIsNotEmpty, ListIndices, DpOfInt, TextUnitOfInt}` are removed from `prism-typed-tree`, with their arms in the validator, the dump, MIR building (`build/call.rs`'s unit construction), the MIR validator and `codec.rs`, and code generation (`lower.rs`'s `ToString` arm).
- Their codec tags (5, 10, 11, 12, 38 and 39) are retired and never reused. No cached record can hold one, since every cache entry is keyed by the compiler identity, so the cache's `FORMAT_VERSION` stays.
- Calls of the Prism members are `Callee::Declaration` calls with a receiver. Dumps print `call prism/core::Int.toString` and `call prism/core::List<T>.isEmpty<Int>` (a generic extension's path names its receiver as declared).
- Fixture snapshots that print the old built-ins are re-blessed, and each diff is checked to differ only by the callee and its receiver borrow (task 5.3).

**Compile-time cost.** None; one node per use as before.

### D12. Extension properties in the ABI (autonomous)

- `ExportedSymbol::ExtensionProperty` gains `kind` (plain or intrinsic) and is exported for every `public` and `package` extension property, not only intrinsic ones.
- Ingestion turns a plain one into an imported extension with `property: true` and no built-in. When it comes from `/prism/core` and its receiver is built in, it becomes a member instead.
- The ABI hash covers the kind, and a getter's body is not in the ABI. So editing a getter recompiles no importer (deterministic hermetic builds), and changing its type re-checks them.
- `prism abi` prints `public val Int.dp: Dp`.
- Slots and contracts (hot reload) are a function's: a getter edit is swapped at the next reload point.

**Compile-time cost.** One more flag per exported property in Phase 1.

### D13. Formatting (autonomous)

`prism fmt` prints the header, then:

- an expression getter on the same line when the whole declaration fits in 100 columns, and otherwise on the next line, indented one level (Kotlin's convention);
- a block getter with `get() {` on the header's line.

The verifier compares the new AST field. No other layout rule changes.

**Compile-time cost.** None for compilation.

### D14. Compile speed and startup (autonomous)

- **New shape.** `compile_timing.rs` gains `extension-properties`: 256, 1,024 and 4,096 functions, each reading a generic and a non-generic user property and the member `indices`, with ×4.6 per ×4 under `PRISM_ASSERT_TIMING=1`.
- **Startup.** Measured on an idle machine before the first implementation commit and after the last, and recorded here:
  - cold and warm with `run_timing` (target 15 ms, D22 of `codegen-core`);
  - `warm_run_timing`'s stages;
  - `ui_run_timing` and `app_frame_timing` (unit reads in frames);
  - `list_timing` (the million adds, 20 ms).
- **Indicative baseline**, taken while proposing on a loaded machine with the release binary of 2026-10-20 08:59: `prism run --timings examples/run/main.prism` reaches `main` in 10.4 ms cold (front end 4.8, code generation 5.1) and 2.1 ms warm.
- **Expected effect.**
  - The front end saves one library module for programs without imports.
  - Cold code generation adds `/prism/core`'s few Prism bodies, once per cache and shared by every workspace, plus one tiny instance per element type that uses `isEmpty` or `indices`.
  - Warm runs load cached units and instances, so no change is expected.
  - A regression of more than 5 % in a cold or warm median is investigated before archiving.
- **Baseline (task 1.2)**, measured on `6600fa53` before any code change, release build, idle machine (load average 1.0 to 1.5, no other build running), 2026-10-20 13:22:
  - `run_timing`, `examples/run/main.prism`, 20 runs: cold total to `main` median 10.55 ms (p90 12.09), front end 4.85 (p90 5.70), code generation 5.31 (p90 5.93), wall 13.60 (p90 15.49); warm total to `main` median 2.06 ms (p90 2.10), front end 1.18, code generation 0.48, wall 4.79 (p90 5.03). Target 15 ms met.
  - `warm_run_timing` (front end / code generation, ms): `many_files` 1024 cold 216.6 / 202.1, warm 78.0 / 20.6, body edit 76.0 / 19.9; `many_instances` 1024 cold 19.5 / 174.0, warm 2.4 / 22.2, generic edit 2.7 / 160.4; `instance_chains` 1024 cold 332.6 / 461.5, warm 79.8 / 59.8.
  - `ui_run_timing`: a click in an app of 1,000 texts, median 0.220 ms (target 1 ms met).
  - `app_frame_timing` (medians, p99): counter click frame 0.003 ms (0.004); todo toggle recomposition 0.153 ms, frame 1.892 ms (p99 3.588); todo typing recomposition 1.499 ms, frame 2.408 ms (p99 4.424, so the "typing frame p99 under 4 ms" target is reported MISSED before this change, a target the test prints but does not assert); todo first frame 29.9 ms cold, 3.0 ms warm.
  - `list_timing`: a million adds, median 7.07 ms (p90 9.45), target 20 ms met.
  - `compile_timing` under `PRISM_ASSERT_TIMING=1`: all three tests pass (253 s); `member scope built once in 0.343 ms`; `library_imports_cost_per_file` at 4,096 files 184.9 ms of imports, 45.1 µs per file, 5.4 µs for members only.
- **Check-cache reuse.** Library entries are keyed as before, and a workspace file's key now carries one member hash. `cache_tests.rs` checks that a second workspace reuses `prism/core.prism` and that a program without imports never mentions `prism/collections.prism`.

### D15. No new error code (autonomous)

Every mistake fits an existing code:

- a malformed property (missing getter, initializer, `var`, a type parameter not in the receiver, `compose` or a receiver mode) is `E2001`;
- an assignment is `E4003`;
- calling a non-function value, or giving `toString` an argument, is `E4004`;
- an unknown property is `E4006`;
- a duplicate, a property shadowed by a field or member, or a workspace extension named like a Prism member is `E4010`;
- an ambiguous import is `E4012`.

The `E2001`, `E4006` and `E4010` rows of `diagnostics` are widened.

## Risks / Trade-offs

- **[Generic members planned for files that do not import `/prism/core`]** A file can call `xs.isEmpty()` with no import of the module declaring it. Instance planning reads the requests from summaries, and the generic record must be found through the member module, an implicit input. → Scenario "Instances of generic members" (`compilation-and-abi`). The reviewer should probe a program that never imports `/prism/core`, with cold and warm caches.
- **[Run-time cost of calls in Tier 1]** `toString`, `isEmpty`, `indices` and units become calls. → Measured (D14). They are not in loops of the reference benchmarks except `isNotEmpty` in `filter`. Tier 2 inlines.
- **[`toString()` receivers]** The built-in copied the scalar out; the member borrows it. A `toString()` on an `Int` held in a `var` that the same expression writes could order differently. → Receivers follow the method-receiver rules every extension already follows. Scenario fixtures cover `x?.toString()`, boxes and references.
- **[Fixture churn]** Typed-tree, MIR and CLIF snapshots naming the removed built-ins, `--explain-cache` outputs listing `prism/collections.prism`, `E4010` messages naming `/prism/collections`, and the renamed scenarios ("A unit property in MIR", "A file's own unit property shadows the import"). → Task 5.3 re-blesses and reads each diff against its scenario. The examples need no edit.
- **[Packets landing in between]** → Task 1.1 rebases the blocks named in the baseline.

## Migration Plan

Programs keep their meaning, apart from D7's shadowing (a file's own `Int.dp` now shadows the import instead of being rejected). Library sources:

- `library/prism/core.prism` gains the list members, the Prism `toString()`s with their externals, and `isEmpty`, `isNotEmpty` and `indices`.
- `library/prism/collections.prism` loses the members and gains `import core`.
- `library/prism/compose/ui.prism` declares `dp` and `sp` with getters.

The build cache is keyed by the compiler identity, so nothing cached is reused across the change. Rollback: revert the commits.

## Open Questions

- Whether `compose-library-in-prism` should be split further (runtime, then state) is for its proposer, once field visibility and the composer primitive are designed (D10).
- Ordinary `var` extension properties with setters, if layout or compose code needs them.
