# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/`, one case per scenario, or one per form of a scenario that has several (`<scenario-slug>--<form>`), as the header of `fixture_tests.rs` describes. Each case runs real `.prism` source:

- `typed-tree` cases hold `typed-tree.snap`, and `mir` cases hold `mir.snap`;
- executable cases hold `stdout.snap` and `stderr.snap` (written with `PRISM_BLESS=1`), and every executable case runs under `--check-allocations`;
- read each snapshot against its scenario's THEN before committing it.

Commit one group at a time, each passing `cargo test --workspace` ("Implement core-library-in-prism: <group>"). Another agent may be working in the same checkout, so stage explicit paths only.

## 1. Reconcile and baseline

- [x] 1.1 Check that `operator-conventions`, `drop-and-clone-hooks` and `vararg` are archived, and stop if one is not.
  - Re-read the main `extension-functions`, `syntax`, `standard-library`, `type-system`, `collections`, `typed-tree`, `mir`, `compilation-and-abi`, `build-cache`, `formatting` and `diagnostics` specs.
  - Rebase every MODIFIED block on the current main text, keeping every scenario of the main requirement. This matters most for the blocks written on other packets' versions (design, "Baseline"): `standard-library` "Intrinsic Operators", "The Collections Module Declares Lists and Their Helpers" and "Intrinsic Calls Are Typed by Their Signatures", `mir` "List Operations Are Built-In Calls" and `typed-tree` "List Operations Are Identified".
  - Turn ADDED blocks whose requirement exists by then into MODIFIED.
  - Check the names the tasks use. In `crates/prism-parser/src/parser.rs`: `parse_extension_property`. In `crates/prism-ast/src/decl.rs`: `ExtensionPropertyDecl`. In `crates/prism-typechecker/src/checker.rs`: `ingest_file_abi`, `register_intrinsic_property`, `register_extension_decl`, `infer_member_access`, `int_property_read`, `builtin_member_call`, `infer_to_string_call`, `has_builtin_to_string`, `has_intrinsic_property`, `has_intrinsic_function`, `member`, `STRING_LENGTH`, `LIST_SIZE` and `LIST_INDICES`. Elsewhere: `MemberScope::from_abis` (`members.rs`), `MEMBER_MODULES` (`crates/prism-library/src/lib.rs`), `member_imports` (`crates/prism-resolver/src/graph.rs`), the unit construction in `crates/prism-mir/src/build/call.rs`, `BuiltinFunction`'s tags in `crates/prism-mir/src/codec.rs`, and the `ToString` arm of `crates/prism-codegen/src/lower.rs`.
  - Get `openspec validate core-library-in-prism --strict` clean with no INFO lines.
  - Record the reconciliation as a "Baseline update" paragraph in `design.md`.
- [x] 1.2 On an idle machine and a release build, before any code change, record in `design.md` (D14) the medians and 90th percentiles of:
  - `cargo test --release -p prism-cli --test run_timing -- --ignored --nocapture` (cold and warm);
  - `--test warm_run_timing`, `--test ui_run_timing`, `--test app_frame_timing` and `--test list_timing`, each with `-- --ignored --nocapture`;
  - `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture`.

  This is the baseline task 7.2 compares with. Verify that the numbers are in `design.md`.

## 2. Syntax and formatting (D5, D13)

- [x] 2.1 AST (`crates/prism-ast/src/decl.rs`): `ExtensionPropertyDecl` gains `getter: Option<Getter>`, an expression or a block body, absent for `intrinsic val`/`var`. Update every match on it: the parser, formatter, ABI extraction, type checker and test helpers. Verify with `cargo build --workspace`.
- [x] 2.2 Parser (`parse_extension_property` and its caller in `parse_declaration`):
  - accept `val` with a receiver and no library modifier;
  - after the type, skip newlines and read `get` `(` `)` then `=` and an expression, or a block, with `get` matched as an identifier at that position only;
  - reject with `E2001`: a missing getter (the message says that an extension property stores no value), an initializer (help `get() = ...`), a getter on an `intrinsic` property, `var` without `intrinsic`, `compose` or a receiver mode before `val`, and a type parameter that does not occur in the receiver type (named in the message).

  Add fixtures under `syntax/` for "Extension property getters parse", "get as an identifier", "Extension properties without a getter rejected", "Intrinsic properties parse" and "Extension properties outside the library form rejected", and parser unit tests for the newline forms. Verify with `cargo test -p prism-lexer -p prism-parser`.
- [x] 2.3 Formatter (`crates/prism-formatter/src/printer/declarations.rs`): print the getter by D13, keeping an expression getter on the header's line inside one group that breaks to an indented `get()` line, and a block getter on the header's line. Extend `verify.rs` to compare the getter. Add fixtures or `fmt_cli_tests.rs` cases for "Getters formatted" and "A getter with a block formatted", and an idempotency check over the new `syntax/` fixtures. Verify with `cargo test -p prism-formatter -p prism-cli --test fmt_cli_tests`.

## 3. ABI (D12)

- [x] 3.1 `crates/prism-abi/src/signature.rs`:
  - `ExportedSymbol::ExtensionProperty` gains `kind` (plain or intrinsic, as `FunctionKind`);
  - Phase 1 exports every `public` and `package` extension property, never a getter's body;
  - the ABI hash and the symbol order cover the kind;
  - `prism abi` prints `public val Int.dp: Dp` for a plain property.

  Update the ABI codec or cache encoding if it serializes these symbols. Add fixtures under `compilation-and-abi/` for "A property's type is part of the ABI", and a `cache_tests.rs` case for "Editing a getter recompiles nothing downstream", which needs two runs. Verify with `cargo test -p prism-abi -p prism-cli --test cache_tests`.

  Implementation note: no codec serializes `ExportedSymbol` (ABIs are re-extracted from source), so none changes. `prism-abi`'s `extension_properties_with_getters_in_the_abi` covers the hash rules at the ABI level here; the fixture and the `cache_tests.rs` case need getters checked and run, so they land with group 4.

## 4. Type checker, typed tree and MIR: extension properties (D6)

- [x] 4.1 Declarations, in `checker.rs`:
  - register a non-intrinsic extension property of the file as an extension with `property: true` and no built-in, in the table extension functions use, keyed by receiver constructor, name and context, so a property and a function of one name are `E4010`;
  - apply `register_extension_decl`'s checks for fields and members (`E4010`, naming the field, or the member's module);
  - check the getter body as an extension body with a shared receiver, the property's type parameters, no parameters and the property's type as its result, not a composable context;
  - in `ingest_file_abi`, turn an imported plain property into an imported extension (today it `continue`s when the catalog has no entry).

  Add fixtures under `extension-functions/` for "A property and a function of one name rejected", "A property named like a field rejected", "A getter calling a composable rejected" and "An unknown property". Verify with `cargo test -p prism-typechecker`.
- [x] 4.2 Reads, in `infer_member_access` and the place and assignment paths:
  - Replace the per-type name tests (`STRING_LENGTH`, `LIST_SIZE`, `LIST_INDICES`, the `Type::Int` branch) by one lookup. A field comes first. Then a member property: its catalog built-in through `builtin_member_call` when it has one, else its declared getter. Then the file's own extension properties, then the imports' (`E4012` when several imports match).
  - Build a getter read as a `Call` of `Callee::Declaration` with `has_receiver`, a shared receiver borrow (`BorrowOrigin::Receiver`) and type arguments solved from the receiver's.
  - `?.` gives `R?`. An assignment or compound assignment is `E4003` ("has no setter"). Parentheses call the value read.

  Add fixtures under `extension-functions/` for "Reading an extension property" (executable), "An imported extension property", "A generic extension property", "Assigning an extension property rejected", "Calling an extension property" and "A getter with a block body" (executable), and under `typed-tree/` for "An extension property read in the typed tree". Verify with `cargo test -p prism-typechecker -p prism-typed-tree`.
- [x] 4.3 MIR (`crates/prism-mir/src/build`): lower a getter's body as an extension body with no parameter, and a read as the `Call` terminator of its path with the receiver borrow first. Check that the borrow checker accepts a getter returning `&T` tied to its receiver by elision rule 2, and rejects using that borrow after the receiver is mutated (`E5003`), with a `references`-style fixture under `extension-functions/` named after "A generic extension property". Verify with `cargo test -p prism-mir -p prism-borrowck`.

## 5. Members of the built-in types in `/prism/core` (D3, D4, D9, D11)

- [x] 5.1 Library:
  - move the `List<T>` member declarations from `library/prism/collections.prism` to `library/prism/core.prism` (with `contains` as `intrinsic operator`);
  - write in Prism `public fun Int.toString(): String = prism_rt_int_to_string(this)`, and the same for `Double`, `Bool` and `Char`, over private externals of the runtime's existing signatures; `public fun <T> List<T>.isEmpty(): Bool = size == 0`; `isNotEmpty()`; and `public val <T> List<T>.indices: Range get() = 0..<size`, each with a comment;
  - give `collections.prism` `import core`;
  - set `MEMBER_MODULES` (`crates/prism-library/src/lib.rs`) to `["prism/core"]`, and follow it through `crates/prism-resolver/src/graph.rs` (`member_imports`), `crates/prism-cli/src/workspace.rs` (check keys) and `compile_timing.rs`'s member ABIs;
  - in `crates/prism-typechecker/src/intrinsics.rs`, change the list entries' module to `prism/core` and remove the `toString`, `isEmpty`, `isNotEmpty` and `indices` entries;
  - make `library_tests.rs`'s `the_member_modules_declare_members_of_built_in_types` check that every extension `/prism/core` declares, plain or intrinsic, has a built-in receiver.

  Verify with `cargo test -p prism-cli --test library_tests`.
- [x] 5.2 Type checker:
  - `MemberScope::from_abis` and `ingest_file_abi` make every extension function or property of `/prism/core` on a built-in receiver a member (`Extension` with no built-in and its declaration path), consulted as members are;
  - remove `infer_to_string_call`, `has_builtin_to_string` and the scalar `toString` branch, so `toString()` takes the member path;
  - make the uncallable-extension check (`E4010`) use the member table, so it covers Prism members and names `/prism/core`.

  Remove `BuiltinFunction::{ToString, ListIsEmpty, ListIsNotEmpty, ListIndices}` from `crates/prism-typed-tree` (tree, validator, dump), `crates/prism-mir` (build, validator, `codec.rs`, retiring tags 5, 10, 11 and 12 with a comment that they are never reused) and `crates/prism-codegen/src/lower.rs`. Verify with `cargo test -p prism-typechecker -p prism-typed-tree -p prism-mir -p prism-codegen`.
- [x] 5.3 Fixtures and snapshots:
  - add or update the cases of `standard-library` ("Members written in Prism run", "Members written in Prism are calls of their declarations", "Unreached modules are not parsed", "An imported module is parsed", "A workspace extension named like a member", "Declarations of the core module", "Members need no import", "A signature-typed intrinsic", "The list membership operator"), `type-system` ("toString takes no arguments" and the other scenarios of "Scalars Have a Built-In toString"), `extension-functions` ("A built-in toString wins over an extension", "Extensions named like built-in members rejected", "A property named like a member rejected"), `collections` (the three MODIFIED requirements' scenarios), `typed-tree` ("Built-in string members", "Built-in members through a box", "Built-in list calls and element places"), `mir` ("String members are built-in calls", "Built-in list calls"), `compilation-and-abi` ("The ABI of a library module", "Member modules give members only", "Instances of generic members") and `build-cache` (`cache_tests.rs`: "Library entries are shared between workspaces", "Explaining a warm check");
  - re-bless every snapshot that prints `builtin toString`, `builtin isEmpty`, `builtin isNotEmpty` or `builtin indices`, or lists `prism/collections.prism` for a program that does not import it, reading each diff to check that only the callee, its receiver borrow and the library lines changed;
  - check that the examples need no edit (`prism check` on every file under `examples/`).

  Verify with `cargo test --workspace`.

## 6. Unit properties (D7)

- [ ] 6.1 `library/prism/compose/ui.prism`: `public val Int.dp: Dp get() = Dp(this)` and `public val Int.sp: TextUnit get() = TextUnit(this)`.
  - Remove the `dp` and `sp` catalog entries, `BuiltinFunction::{DpOfInt, TextUnitOfInt}` (retiring codec tags 38 and 39), the unit construction in `crates/prism-mir/src/build/call.rs`, and `int_property_read`'s use for them.
  - Rename and re-bless the fixtures: `mir/a-unit-in-mir` becomes `mir/a-unit-property-in-mir`, and the `standard-library` case of "Unit properties are members of Int" becomes "A file's own unit property shadows the import" (executable).
  - Update "The public signature of runApp", and check that `explicit-library`'s import help still names `/prism/compose/ui` for `16.dp` without the import ("An extension of a built-in type from another module is imported", `crates/prism-cli/src/library_index.rs`).
  - Run every UI entry with `prism run --check-allocations --headless` (`examples/apps/counter/main.prism`, `examples/apps/todo/main.prism`, `examples/apps/todo/large.prism`, `examples/demo/ui/ui.prism`): status 0, empty stderr, and the `example_app_sessions` sessions unchanged.

  Verify with `cargo test --workspace`.

## 7. Compile timing and measurements (D14)

- [ ] 7.1 Add the `extension-properties` shape to `crates/prism-cli/tests/compile_timing.rs` (D14), which is the scenario "Many property reads check in linear time". Run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` on an idle machine, and record the times per size in `design.md`.
- [ ] 7.2 Re-run every measurement of task 1.2 on an idle machine and record them beside the baseline in `design.md`. Investigate any cold or warm median more than 5 % slower before going on, rerunning first since concurrent builds inflate timings (`docs/agent-workflow.md` §5). Verify that `run_timing`'s cold run stays under 15 ms and `list_timing` under 20 ms.

## 8. Documents and checks

- [ ] 8.1 Make targeted edits:
  - `openspec/specs/diagnostics/spec.md`: widen the `E2001` row (extension property declarations), the `E4006` row (unknown properties) and the `E4010` row (members written in Prism, `/prism/core` as the one member module, properties duplicating functions, fields or members; `Int.dp` no longer a member).
  - `AGENTS.md`'s reference list: `extension-functions` gains extension properties; `standard-library` says `/prism/core` declares the built-in types' members, intrinsic or written in Prism.

  Verify that `openspec validate core-library-in-prism --strict` stays clean.
- [ ] 8.2 Run the apply checks of `openspec/config.yaml`:
  - `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`;
  - `prism check` on every file under `examples/`, and `prism fmt --check examples/`;
  - `prism run --check-allocations examples/run/main.prism`, and the four UI entries with `--check-allocations --headless`.

## Workflow follow-up

- Review with a fresh agent, using adversarial probes:
  - generic members (`isEmpty`, `indices`) used by files that never import `/prism/core`, with cold and warm caches and through generic library helpers (`filter`);
  - `toString()` on references, boxes, `?.` chains and temporaries;
  - getters returning borrows, then a mutation of the receiver;
  - a file's own `dp` beside the import;
  - a property and an extension function of one name across two imports;
  - hot reload of a getter body;
  - formatter idempotency on long getters.
- Then archive per `openspec/config.yaml`:
  - promote D2 to D9 and D11 to D15 into the Design Rationale of `standard-library`, `extension-functions`, `syntax` and `compilation-and-abi`;
  - add the deferrals to `ROADMAP.md` as unchecked items naming this change: `var` extension properties with setters; `compose` and context-parameter properties; `String.length`, `List.size`, `removeLast` and `clear` in Prism once inlining or `unsafe` exists; `print`/`println` as Prism once `Any` or traits exist;
  - tick the `standard-library` follow-up "Extension properties outside the library" and sub-item `core-library-in-prism` of item 10 in "After Milestone 2";
  - point `compose-library-in-prism`'s proposer at D10.
