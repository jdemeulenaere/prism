# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/`, one case per scenario, or one per form of a scenario that has several (`<scenario-slug>--<form>`), as the header of `fixture_tests.rs` describes. Each case runs real `.prism` source:

- library test modules use `// library:` cases, which are check-only;
- `typed-tree` cases hold `typed-tree.snap`, and `mir` cases hold `mir.snap`;
- executable cases hold `stdout.snap` and `stderr.snap` (written with `PRISM_BLESS=1`), and every executable case runs under `--check-allocations`;
- read each snapshot against its scenario's THEN before committing it.

Commit one group at a time, each passing `cargo test --workspace` ("Implement compose-library-in-prism: <group>"). Another agent may be working in the same checkout, so stage explicit paths only.

## 1. Reconcile and baseline

- [x] 1.1 Check that `drop-and-clone-hooks`, `vararg`, `core-library-in-prism` and `field-visibility` are archived, and stop if one is not.
  - Re-read the main `syntax`, `formatting`, `standard-library`, `compose`, `mir`, `typed-tree`, `code-generation`, `compilation-and-abi`, `ui`, `references` and `diagnostics` specs.
  - Rebase every MODIFIED and REMOVED block on the current main text, keeping every scenario of the main requirement. This matters most for the blocks written on `core-library-in-prism`'s versions (design, "Baseline"): `standard-library` "Library Modules Compile Like Workspace Files" and the REMOVED "The UI Intrinsics".
  - Turn ADDED blocks whose requirement exists by then into MODIFIED.
  - Check the names the tasks use:
    - `IntrinsicStruct` and `Type::contains_keeper` (`crates/prism-typed-tree/src/types.rs`);
    - the `E4030` list in `checker.rs`;
    - `is_composition_operation`, `uses_composition_operations`, `Rewriter::intrinsic`, `adapter_value`, `state_object` and `node` (`crates/prism-compose-transform/src/rewrite.rs`), `root_content_body` (`restart.rs`), and `stability.rs`'s `WeakReference` id;
    - `BodyOwner::RootContent` and `ComposerOperation` (`crates/prism-mir/src/mir.rs`) and their codec tags;
    - the `RunApp` arm of `crates/prism-codegen/src/lower.rs`, and the `Composition` and equality-entry cases of `glue.rs`;
    - `prism_compose_start`, `prism_compose_set_content` and `prism_ui_run_app` in `crates/prism-compose/src/entry.rs` and `crates/prism-ui/src/entry.rs`.
  - Get `openspec validate compose-library-in-prism --strict` clean with no INFO lines.
  - Record the reconciliation as a "Baseline update" paragraph in `design.md`.
- [x] 1.2 On an idle machine and a release build, before any code change, record in `design.md` (D15) the medians and 90th percentiles of:
  - `cargo test --release -p prism-cli --test run_timing -- --ignored --nocapture` (cold and warm);
  - `--test warm_run_timing`, `--test ui_run_timing`, `--test app_frame_timing` and `--test list_timing`, each with `-- --ignored --nocapture`;
  - the `many-composables`, `composable-calls`, `ui-calls` and `nested-compose-ui` shapes of `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture`.

  This is the baseline task 7.3 compares with. Verify that the numbers are in `design.md`.

## 2. Annotations (D2, D12, D16)

- [x] 2.1 Lexer and AST: an `@` token, and `annotations: Vec<Annotation>` (name and span) on every top-level declaration. Parser: annotations before `context(...)`, the visibility and the other modifiers, on the same line or their own. `E2001` for an `@` not followed by an identifier, arguments, and annotations elsewhere. Add fixtures under `syntax/` for "Annotations before declarations" and "Malformed annotations rejected". Verify with `cargo test -p prism-lexer -p prism-parser`.
- [x] 2.2 Formatter: annotations on their own lines above the declaration, in order, with comments kept above the first. `verify.rs` compares them. Add a case for "Annotations formatted", and an idempotency check. Verify with `cargo test -p prism-formatter -p prism-cli --test fmt_cli_tests`.
- [x] 2.3 Checks:
  - Phase 1 reports `E3002` for any annotation in a workspace file, at its `@`.
  - The type checker reports `E4029` in library modules for an unknown annotation, a wrong target, a repeat, and `@DrivesComposition` on a composable.
  - Annotation flags are recorded in `TypeTable` (keeper) and on function signatures (drives composition, explicit groups).

  Add fixtures for "Annotations outside the bundled library rejected" (`standard-library/`) and "Misused annotations in the library".
- [x] 2.4 ABI (D12): `ExportedSymbol::Struct` and `AbiTypeDefinition` gain `keeper`, and `ExportedSymbol::Function` gains `drives_composition`, all hashed. Importers fill `TypeTable` from both sources, as for `equality` and the hook flags. `prism abi` prints the annotations. Add cases for "Annotations change the ABI hash" and, after group 5, "A keeper reached without its module". Verify with `cargo test -p prism-abi -p prism-cli --test cache_tests`.
- [x] 2.5 Make the keeper and `E4030` read the flags instead of names:
  - `Type::contains_keeper` reads the keeper flag through the type table;
  - `E4030` is reported for direct calls of functions with the drives-composition flag, naming the function.

  Keep `IntrinsicStruct::is_keeper` and the `E4030` list until groups 5 and 6 annotate the library, then delete them. Add fixtures for "A keeper declared by its annotation" and "A function that drives compositions". Verify with `cargo test --workspace`.

## 3. The composer primitive (D3, D4)

- [x] 3.1 `currentComposer`:
  - catalog entry `public intrinsic compose fun currentComposer(): Int` of `prism/composer`, declared in `library/prism/composer.prism`;
  - `BuiltinFunction::CurrentComposer` with a codec tag and dump name;
  - the transform lowers it to a copy of `$composer`, and treats it as no composable call for regions and call-site groups.

  Add fixtures for "Protocol calls inside a composable" (`standard-library/`, executable), "The current composer in the tree" (`typed-tree/`) and "The composer read in a composable" (`mir/`).
- [x] 3.2 Runtime entries:
  - `prism-compose` gains `prism_compose_start_composable(content) -> u64` and `prism_compose_set_composable(composition, content)`, listed in `COMPOSE_FUNCTIONS`. A root records whether its content is word content or composable content, and calls composable content through its descriptor with the composer and the force bit (1).
  - `prism_ui_run_app` takes composable content and starts its composition through the same path.
  - Rust tests cover both kinds of root content, disposal and release order.

  Verify with `cargo test -p prism-compose -p prism-ui`.
- [x] 3.3 Check that externals with composable function parameters pass `E6005`'s signature check (an address), and add the case for "Content given to an external" (a `// library:` case dumping the module's CLIF).

## 4. Composables with explicit groups (D5)

- [x] 4.1 Type checker:
  - a body annotated `@ExplicitGroupsComposable` gets `E4029` for a composable call, outside closures, in a branch, loop, `&&`/`||` right operand, `?:` fallback, safe call or `when` with composable tests, and for `remember` and `key`;
  - the annotation is recorded on the MIR body.

  Add the fixture for "Grouped code in a body with explicit groups rejected".
- [x] 4.2 Transform: such a body gets no body group, no keep, no skipping and no restart closure. Its calls pass its composer and its change words. Add the fixture for "A node function's body" (`mir/`). Verify with `cargo test -p prism-compose-transform -p prism-cli --test fixture_tests`.

## 5. The runtime module in Prism (D6, D7, D13)

- [x] 5.1 `library/prism/compose/runtime.prism`: write `Composition` (`@Keeper`, private `handle`, drop hook), `composition`, `setContent` and `recompose` (`@DrivesComposition`), `dump`, `RecomposeScope` (private `word`), `invalidate`, `currentRecomposeScope` and `SideEffect` (`@ExplicitGroupsComposable`), as in design D6 and D7. Use private externals of the compose runtime and `import /prism/composer` for `currentComposer`, and check that `library_tests.rs` accepts an external declared by two modules. Remove their catalog entries, and `IntrinsicStruct::{Composition, RecomposeScope}`.
- [x] 5.2 Remove:
  - `BuiltinFunction::{Composition, SetContent, Recompose, DumpComposition, InvalidateScope, SideEffect, CurrentRecomposeScope}`;
  - `ComposerOperation::{StartComposition, SetContent, Recompose, Dump, Invalidate, SideEffect, CurrentScope}`;
  - `BodyOwner::RootContent`, `root_content_body`, `adapter_value`, `is_composition_operation` and `uses_composition_operations`;
  - `Composition`'s glue special case.

  Retire their codec tags. The transform now rewrites composable bodies only.
- [x] 5.3 Add fixtures for:
  - "A side effect adds no group", "A side effect never skips", "A composable function as root content", "A composition cannot be forged" and "Members of a composition" (`compose/`, `standard-library/`);
  - "A composition started in MIR" (`mir/`) and "A side effect in the tree" (`typed-tree/`);
  - "Layout of a composition" and "The drop glue of a composition calls its hook" (`code-generation/`).

  Re-bless the dumps that showed root content adapters or the removed operations, and check each diff against D14. Verify with `cargo test --workspace`.

## 6. The UI module in Prism (D8 to D10, D13)

- [x] 6.1 `library/prism/compose/ui.prism`:
  - `NodeId` and `HandlerId` as structs over a `package` word, and `HandlerId.equals`;
  - the UI externals take and return `Int` words;
  - `Node` and `LeafNode` with `@ExplicitGroupsComposable`, starting node groups with key 125 through private externals of the compose runtime;
  - `runApp` with `@DrivesComposition` over `prism_ui_run_app`;
  - `import /prism/composer`.

  Update `foundation.prism`, `layout.prism` and `material3.prism` (`node.word`, `HandlerId(...)` around registrations).
- [x] 6.2 Remove:
  - `IntrinsicStruct::{NodeId, HandlerId}` and the equality entry;
  - `BuiltinFunction::{RunApp, UiNode, UiLeafNode}` and `ComposerOperation::StartNodeGroup`, with their transform, codec and backend arms.

  Make signatures pass only the three state handles as words. Add fixtures for:
  - "The public UI declarations", "Calling runApp with its defaults" and "A handler id cannot be made by a program" (`standard-library/`);
  - "A toolkit composable calls its node function" (`mir/`);
  - "Comparing handler ids" and "A weak reference passed as a word" (`code-generation/`);
  - "Inserting a button before a text keeps the text's node" (`ui/`, kept).
- [x] 6.3 Run every `ui/` fixture, the example sessions (`example_app_sessions`) and `library_tests.rs`. Re-bless only the dumps D14 lists, and read each diff. Verify with `cargo test --workspace`.

## 7. Catalog, timing and verification (D11, D14 to D16)

- [ ] 7.1 Catalog (D11):
  - `IntrinsicStruct` holds `State`, `MutableState` and `WeakReference`, and `IntrinsicStruct::of` is the only comparison of a library declaration's module and name in the compiler;
  - `stability.rs` builds `WeakReference`'s id through the catalog;
  - the compose function catalog holds the 10 entries of D11.

  Grep the type checker, borrow checker, transform and backend for the removed names, and record what remains in "Implementation Notes".
- [ ] 7.2 Diagnostics catalog (`openspec/specs/diagnostics/spec.md`): widen `E2001` (malformed annotations), `E3002` (annotations in the workspace), `E4029` (annotation rules and explicit-groups bodies) and `E4030` (`@DrivesComposition` functions).
- [ ] 7.3 Timing (D15): on an idle machine, re-run the measurements of task 1.2 and record them beside the baseline in `design.md`. Investigate any regression of more than 5 % in a cold, warm or frame median before archiving, and check the four compile-timing shapes stay within ×4.6 per ×4.
- [ ] 7.4 Run the apply checks of `openspec/config.yaml`:
  - `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`, reading the `test result:` lines;
  - `prism check` on every file under `examples/`, and `prism fmt --check examples/`;
  - `prism run --check-allocations examples/run/main.prism`, and each UI entry with `--headless` (status 0, empty stderr).
- [ ] 7.5 Update `ROADMAP.md`:
  - tick item 10c, and item 10 once all its sub-items are done;
  - mark the `drop-and-clone-hooks` follow-up "Replace `Composition`'s compiler drop ... by hooks written in Prism" done for `Composition`;
  - add the follow-ups of the proposal's "Not in scope" list and of design "Open Questions" as unchecked items naming this change.

  In `docs/milestone-2-review-notes.md` (A4), record the compose half as done.
