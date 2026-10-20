# Proposal: compose-library-in-prism

## Why

The post–Milestone 2 review agreed that the standard and compose libraries become "as much as possible ... normal prism code that is imported like any other code" (item **A4** of `docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20). The core of the compiler shrinks to a minimal set, which includes "one composer-access primitive". There is no `unsafe` layer, so runtime state stays behind externals.

`core-library-in-prism` (item 10a of "After Milestone 2: the review round" in `ROADMAP.md`) did the core and collections half and classified the compose half in its D10. This change, item 10c, does the compose half. It builds on four earlier changes:

- `field-visibility` (10b): library structs can wrap a runtime handle without being forged;
- `drop-and-clone-hooks`: such a struct can release its handle when dropped;
- `operator-conventions`: a type can declare its own `==`;
- `vararg`.

Today the whole of `/prism/compose/runtime` and the toolkit handles of `/prism/compose/ui` are intrinsics:

- 7 intrinsic structs: `Composition`, `RecomposeScope`, `State`, `MutableState`, `WeakReference`, `NodeId` and `HandlerId`;
- 19 intrinsic functions and properties, once `core-library-in-prism` has moved `Int.dp` and `Int.sp`.

The compiler also recognizes several of them by name, in five places:

- the type checker's `E4030` list;
- the borrow checker's keeper test (`Composition`);
- the compose transform's dispatch, with root content adapters for `composition`, `setContent` and `runApp`;
- the code generator's `Composition` drop and `HandlerId` equality;
- `runApp`'s lowering.

Jetpack Compose writes almost all of this in library code over `currentComposer`. Its compiler knows `@Composable`, `key` and a few annotations.

## What Changes

- **The composer primitive.** It has two halves.
  - `currentComposer()`, a composable intrinsic of `/prism/compose/runtime` and Compose's `currentComposer`, gives the running body's composer word (placed in the protocol module `/prism/composer` when proposed, moved at implementation: design D3). The transform reads it from the body's composer argument.
  - Composable content (`compose () -> Unit`) can be given to externals. The compose runtime runs it for a composer by calling it with that composer and the force bit, as Compose's runtime does in `invokeComposable`. The transform's root content adapters (`<symbol>#root-content`) are removed.
- **Library annotations**, Kotlin's syntax `@Name` before a declaration, as the declared markers that replace the name tests. All are library-only for now: a workspace file using one gets `E3002`.
  - `@ExplicitGroupsComposable` (Compose's name): a composable that starts no group of its own, never skips and is no restart scope, so it runs in its caller's group. Its body makes no composable call inside a branch or loop and uses no `remember` or `key` (`E4029`).
  - `@Keeper` marks a struct whose values keep the `attached` arguments of the calls that make or change them. It replaces the borrow checker's test of `Composition` (`owned-state` D24).
  - `@DrivesComposition` marks a function that runs composition passes, so calling it in a composable context is `E4030`. It replaces the type checker's list of `composition`, `setContent`, `recompose` and `runApp`.
- **Written in Prism** (`core-library-in-prism` D10, challenged where Compose does it in library code):
  - `Composition`: a `@Keeper` struct with a private handle and a drop hook that disposes it. `composition`, `setContent`, `recompose` and `dump` are written over runtime externals that take composable content.
  - `RecomposeScope`: a `Copy` struct with a private word. `invalidate()` is Prism, and `currentRecomposeScope()` is an `@ExplicitGroupsComposable` over `currentComposer()`.
  - `SideEffect`: an `@ExplicitGroupsComposable` that queues its effect through a runtime external. Without the annotation, strong skipping would skip it.
  - `NodeId` and `HandlerId`: `Copy` structs over a `package` word. `HandlerId`'s runtime equality becomes `operator fun HandlerId.equals`. The UI externals take and return `Int` words.
  - `Node` and `LeafNode`, which D10 kept intrinsic: `@ExplicitGroupsComposable` functions that start a node group through the protocol, as Compose's `ComposeNode` is library code over `currentComposer`. Their node groups take one constant key (Compose's node key) and match among their siblings in order, so a node keeps its identity as before.
  - `runApp`: a `@DrivesComposition` function over `prism_ui_run_app`, which now takes the composable content.
- **Stays compiler-known**, each reached only through its declaration's catalog entry:
  - `remember`: keys need overloading, its result is a copy or a borrow of the slot that no signature states without lifetimes, it gives permanent identities, and it marks state windows;
  - `key`: Compose's compiler treats it too;
  - `currentComposer`;
  - the state core: `State`, `MutableState`, `mutableStateOf`, `value`, `update`, `weak` and `get`;
  - `WeakReference`, which D10 proposed to move. Its capture function and `get` stay intrinsics, and the compiler builds its values for weak captures.

  The intrinsic struct catalog drops from 7 entries to 3, and the compose part of the function catalog from 19 entries to 10.
- **Compiler simplifications.**
  - The transform no longer rewrites bodies that are not composable.
  - The `BuiltinFunction`, `ComposerOperation` and `IntrinsicStruct` variants of the moved declarations are removed, with their MIR codec tags retired.
  - `BodyOwner::RootContent` is removed.
  - Only `State`, `MutableState` and `WeakReference` still travel as one machine word.
- **Behaviour is unchanged**: every fixture and example keeps its output, and executable fixtures run under `--check-allocations`. The accepted differences:
  - dumps: no `#root-content` closures, calls of the library composables as ordinary calls, node groups keyed by the node key;
  - `prism abi` of the two modules;
  - `--explain-cache` listing `prism/compose/runtime.prism` for programs importing the UI modules, since `/prism/compose/ui` imports it for `currentComposer`;
  - a file's own extension named like a moved member (`fun Composition.recompose()`) now shadows the import instead of being `E4010`.
- **No new error code**: `E3002`, `E4029`, `E4030` and `E2001` are widened.
- **BREAKING (small)**: as listed under behaviour. A workspace file can no longer construct or read the handle structs (`E4007`), which were already unconstructible as intrinsics.

Not in scope, left for follow-ups:

- annotations in workspace code (`@NonRestartableComposable`, `@Stable`, ...);
- `remember` in Prism, once overloading exists;
- a typed `Composer`;
- subcompositions;
- moving the state core off the compiler, which needs `unsafe` or generic externals.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `syntax`: annotations before top-level declarations.
- `formatting`: annotations are printed on their own lines.
- `standard-library`: the library annotations and their checks, `currentComposer`, externals taking composable content, the compose module and the UI modules' declarations written in Prism, what stays intrinsic, and the modules a program parses.
- `compose`: composables with explicit groups, root content run with the force bit, compositions as library values, and `E4030` from `@DrivesComposition`.
- `mir`: no root content adapters, the current composer as the body's argument, bodies with explicit groups, and library composables called as ordinary functions.
- `typed-tree`: `SideEffect` is no longer a built-in, and `currentComposer` is one.
- `code-generation`: compositions and scopes as library structs, the one-word list reduced to the state types, and handler ids compared through their `equals`.
- `compilation-and-abi`: the library annotations in the ABI, and the intrinsic structs left.
- `ui`: node identity under the node key.

## Impact

- `crates/prism-lexer` (the `@` token), `crates/prism-ast` (annotations on declarations), `crates/prism-parser`, `crates/prism-formatter`.
- `crates/prism-abi/src/signature.rs`: `keeper` on structs and reachable definitions, `drives_composition` on functions, hashed and printed.
- `crates/prism-typechecker`:
  - `intrinsics.rs`: catalog entries removed; `CurrentComposer` added;
  - `checker.rs`: annotation checks, `E3002` and `E4029`, `E4030` from the flag, the explicit-groups body check;
  - `types.rs`: keeper and annotation flags in `TypeTable`.
- `crates/prism-typed-tree/src/types.rs`: `IntrinsicStruct` keeps `State`, `MutableState` and `WeakReference`; `contains_keeper` reads the flag.
- `crates/prism-borrowck`: keeper from the flag.
- `crates/prism-mir`: `BuiltinFunction`, `ComposerOperation` and `BodyOwner::RootContent` variants removed; codec tags retired.
- `crates/prism-compose-transform`: `rewrite.rs` (no adapters, no composition operations, `currentComposer`, explicit-groups bodies), `restart.rs` (`root_content_body` removed) and `lib.rs`.
- `crates/prism-codegen`: `lower.rs` (no `RunApp` arm), `glue.rs` (no `Composition` special case, no equality entry), `layout.rs` and the signature code (one-word structs).
- `crates/prism-compose` (`prism_compose_start_composable`, `prism_compose_set_composable`; roots called with the force bit) and `crates/prism-ui` (`prism_ui_run_app` taking composable content).
- `library/prism/composer.prism`, `library/prism/compose/runtime.prism`, `library/prism/compose/ui.prism`, `foundation.prism`, `layout.prism` and `material3.prism`: node and handler words.
- Fixtures under `crates/prism-cli/tests/fixtures/{syntax,formatting,standard-library,compose,mir,typed-tree,code-generation,compilation-and-abi,ui}/`, re-blessed dumps, `library_tests.rs`, `cache_tests.rs`, and the timing tests (`run_timing`, `warm_run_timing`, `ui_run_timing`, `app_frame_timing`, `compile_timing`).
- `openspec/specs/diagnostics/spec.md` (`E2001`, `E3002`, `E4029` and `E4030` rows), `ROADMAP.md` (item 10c), and `docs/milestone-2-review-notes.md` (A4).
