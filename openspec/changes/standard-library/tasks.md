# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/<scenario-slug>[--<form>]/`, as the header of `fixture_tests.rs` describes. Each scenario of the deltas gets a case running real `.prism` source; library test modules come from the `// library:` header of task 7.5. Scenarios whose WHEN names a Rust test or a compiled unit are tested where their task says.

Prerequisites (design, "Baseline"): `monomorphization`, `instance-cache`, `closures`, `escaping-closures`, `collections` and `hot-reload` are archived. Before starting, compare the MODIFIED blocks with the main specs as those changes left them, and reconcile any drift:
- `build-cache`: "Cache Keys Cover Exactly the Compilation Inputs" and "Explaining Cache Use";
- `code-generation`: "Calls Go Through the Function Table";
- `syntax`: "Contextual Keyword Identifier Permissibility", "Lexical Tokenization and Escape Sequences" and "Receiver Mode Modifiers";
- `collections`: "List Members Are Typed as Extensions" and "List Members Take Precedence Over Extensions";
- `extension-functions`, `type-system` and `execution`: the requirements their deltas modify.

Re-read the diagnostics catalog for the next free codes: `E1008`, `E3002`, `E4029` and `E6005` are assumed free.

## 1. Syntax: library modifiers, body-less declarations, intrinsic properties (D6, D12)

- [ ] 1.1 In `crates/prism-ast/src/decl.rs`, add `DeclarationBody` (`Prism(FunctionBody)`, `Intrinsic`, `External`), replacing `FunctionDecl::body: Option<FunctionBody>`, with the modifier's span kept for diagnostics. Add `Declaration::ExtensionProperty(ExtensionPropertyDecl { visibility, type_parameters, receiver, name, ty, span })`. Update every match: `prism-abi`, `prism-typechecker`, `prism-formatter`, `prism-cli`. Verify with `cargo build --workspace` and the existing tests.
- [ ] 1.2 In `crates/prism-parser/src/parser.rs::parse_declaration`, read `external` and `intrinsic` as contextual modifiers after the visibility, directly before `compose`, `mut`, `consuming`, `fun` or `val`, as `consuming` is read (around the `at_consuming` test). In `parse_function`:
  - report `E2001` for a body after either modifier;
  - report `E2001` for a missing body without one ("a function needs a body");
  - report `E2001` for both modifiers, or one twice, and for the wrong orders of the scenario.

  Parse `intrinsic val [<T>] Receiver.name: Type` with no initializer. Leave `val Receiver.name` without `intrinsic` an `E2001`. Verify with parser unit tests and `syntax` fixtures for every scenario of "Library Declaration Modifiers", "Function Bodies Are Required", "Intrinsic Extension Properties" and "external and intrinsic as identifiers".
- [ ] 1.3 Update the span test of "Declaration and argument spans" (parser tests) to `intrinsic fun a(): Int`, and check that the other scenarios of "Lexical Tokenization and Escape Sequences" still pass.
- [ ] 1.4 In `crates/prism-formatter/src/printer/declarations.rs::function`, print the modifiers in canonical order and no body for intrinsic and external functions. Add the printing of extension properties. Have `verify.rs` compare the new AST fields. Verify with a `formatting` fixture for "Library declarations formatted", and with the formatter's idempotency test over the library sources once they exist (task 2.2).

## 2. The library crate and the resolver (D1, D2, D4, D15)

- [ ] 2.1 Create `crates/prism-library` (no dependencies), with `src/lib.rs` holding `LibraryModule { path, source }`, `MODULES` (`include_str!` of each file), `PRELUDE = ["prism/core", "prism/collections"]` and `Library`:
  - `bundled() -> &'static Library`;
  - `module(path) -> Option<&LibraryModule>`;
  - `prelude() -> &[&str]`;
  - `with_modules(extra: Vec<(String, String)>) -> Library`, documented as test-only.

  Add it to the workspace `Cargo.toml` and as a dependency of `prism-resolver` and `prism-cli`. Verify with unit tests of the lookup and of `with_modules` replacing or adding a module.
- [ ] 2.2 Write `crates/prism-library/prism/core.prism`: `package prism`, then the public intrinsics of "The Core Module" with a comment above each saying what it does (for `print`/`println`: the text-form rule and the optional argument). Write `collections.prism`: `listOf` (commented as taking any number of elements) and every member of `collections` as a public intrinsic extension function or property, with the receiver modes and signatures of `collections` D5. Write `system.prism` with `exitProcess` over `private external fun prism_rt_exit(status: Int): Nothing`. Leave the helpers to group 8. Verify that each file parses and that `cargo run -q -p prism-cli -- fmt --check crates/prism-library/prism` lists nothing.
- [ ] 2.3 In `crates/prism-resolver/src/graph.rs`:
  - add `SourceOrigin::{Workspace, Library}` and `prelude_imports: Vec<PathBuf>` to `SourceNode`;
  - give `DependencyGraph::from_entries` a `&Library` (`from_entries_with_library`; `from_entries` passes `Library::bundled()`);
  - load library nodes from `Library` under the virtual key `prism/<name>.prism`, never from disk, and skip root discovery and the `E1004` comparison for them;
  - give every workspace node the prelude modules as `prelude_imports` and push them on the worklist;
  - include `prelude_imports` in `compute_sccs`' edges.

  Add `DependencyGraph::from_library(&Library)`, a graph of every library module, for task 8.3. Do the same for `hot-reload`'s `from_entry_with`, whose `SourceStore` must never hold or invalidate library nodes. Verify with unit tests: a file without imports has two prelude nodes, a library node is never read from disk, and the order puts library nodes first.
- [ ] 2.4 In `crates/prism-resolver/src/import.rs::resolve_import`, resolve a root-anchored import of two or more segments starting with `prism` to the library module, or `FileNotFound` (`E1002`) naming the bundled library. For a library file, resolve sibling and `/prism/...` imports in the library and reject any other with `E1002` ("library files import only library modules"). Add `ResolveError::ReservedPackage` (`E1008`, help "rename the package") in `error.rs`, raised in `from_entries` for a workspace file whose package's first segment is `prism`. Verify with `package-resolution` fixtures for "A workspace package named prism rejected" (both forms), "A workspace directory named prism is not imported", "A root file named prism is an ordinary module", "Import of a library module" and "A library file imports a workspace module".
- [ ] 2.5 In `crates/prism-cli/src/workspace.rs`, skip library nodes in `assert_workspace_boundary`. Make `module_path` read a library node's path from its key. Make every error path take the source from `SourceNode::source` instead of `fs::read_to_string` (in `validate_and_extract_abi`, `elaborate_file` and `extract_abis`), and the same in `run.rs`. Make `diagnostics::display_path` show a library key as is. Verify with the fixture "A diagnostic in a library file" (task 7.5) and the existing diagnostics tests.

## 3. Phase 1: library declarations in the ABI (D3, D12)

- [ ] 3.1 In `crates/prism-abi/src/signature.rs`, add a `kind: FunctionKind::{Prism, Intrinsic, External}` to `ExportedSymbol::Function` and a new `ExportedSymbol::ExtensionProperty`. Hash both in the ABI hash (`hasher.rs` tags), render them in `prism abi` with their modifiers, and include property types in `abi_symbol_types`. Verify with unit tests: the rendering of `public intrinsic val <T> List<T>.size: Int`, and the hash changing when only the kind changes ("The ABI of a library module").
- [ ] 3.2 Give `FileAbi::extract_with_definitions` an `in_library: bool`. When it is false, report `AbiError::LibraryOnlyDeclaration` (`E3002`) at the first `external` or `intrinsic` modifier, with help "only the bundled library may declare external or intrinsic functions". Pass the node's origin from `workspace.rs::extract_abis`. Verify with the `standard-library` fixture "External declarations outside the bundled library rejected" (both forms).

## 4. Type checker: prelude scope, intrinsic catalog, members, externals (D4 to D7, D9)

- [ ] 4.1 Add `crates/prism-typechecker/src/prelude.rs` with `PreludeScope`, built once from the prelude modules' `FileAbi`s (`PreludeScope::from_abis`, reusing `ingest_file_abi`'s conversions). It holds:
  - names (symbols with their module);
  - non-member extensions keyed by `ReceiverKey` and name;
  - members (intrinsic extension functions and properties) keyed the same way, each an `Extension` with its `BuiltinFunction`;
  - the prelude's type definitions and markers.

  Add `TypeChecker::with_prelude(Arc<PreludeScope>)`. Verify with unit tests that building it twice from the same ABIs gives equal scopes and that a lookup costs no ingestion.
- [ ] 4.2 Consult the prelude last:
  - in name lookup (`ScopeStack` lookups and `lookup_import` callers), after locals, the file and explicit imports;
  - in `find_extension`, after own and imported extensions, with no `E4012` between an import and the prelude;
  - in the `E4010` duplicate check for top-level declarations, never.

  Library files get no prelude (`typecheck_file` passes it only for workspace nodes). Verify with fixtures for every scenario of "Prelude Names Are Found Last", "Prelude Extensions Are Found After Imported Ones", "Library Files Import What They Use" and "Prelude extensions need no import".
- [ ] 4.3 Add `crates/prism-typechecker/src/intrinsics.rs`: the catalog from (module, receiver type constructor, name) to `BuiltinFunction`, the canonical signature text, and whether the intrinsic is rule-typed. Cover `print`, `println`, `Int`/`Double`/`Bool`/`Char.toString`, `String.length`, `listOf` and every list member. When checking a library file, render each intrinsic declaration's signature as `prism abi` does, compare it with the catalog, and report `TypeError::InvalidLibraryDeclaration` (`E4029`) for an unknown path or a different signature, showing the expected one. Verify with fixtures (task 7.5) for "An unknown intrinsic is rejected" and "An intrinsic with another signature is rejected".
- [ ] 4.4 Replace the special cases with the catalog and the prelude scope:
  - `output_builtin` and `output_builtin_named` (calls of the prelude's `print`/`println` route to `infer_output_call` by `BuiltinFunction`);
  - the scalar `toString()` branch of `infer_method_call`;
  - the `length` branch of `build_member_access`;
  - `collections`' list-member table and its `listOf` lookup.

  Make members a lookup in the scope's member map in `infer_method_call` and `build_member_access`, after fields and the built-in `clone()` and `unbox()`. Make the `E4010` uncallable-extension check consult the member map instead of its hand-written names, naming the member's module in the message. Verify that every existing `type-system`, `extension-functions` and `collections` fixture passes, plus new fixtures for "A workspace extension named like a member", "A workspace extension named like a helper", "A signature-typed intrinsic", "A rule-typed intrinsic", "Core names need no import" and "Prelude intrinsics are only called".
- [ ] 4.5 Factor the text-form check of `check_template_part` into `check_text_form(part, context)`, and make `check_printed_argument` use it: scalars and strings as today, other types through the resolved `toString()` on a shared borrow into a statement temporary, and `E4026` with the template help for the rest. Widen the `NoTextForm` message to say "printed value" for print arguments. Verify with `type-system` fixtures for every scenario of "Output Built-In Arguments" (the typed-tree snapshot of "Printing a value through its toString" shows the `toString()` call), and check that the existing typed-tree snapshots of `print` calls are unchanged.
- [ ] 4.6 Check external declarations in library files: report `E4029` for `public`, type parameters, a receiver, `compose`, default values and a result type holding a reference, each naming its rule. Record calls of externals as `Callee::External(DeclarationPath)` (task 5.1). Verify with fixtures for "Externals that cannot be declared" (five cases in one module).
- [ ] 4.7 Add a test helper, `prism_typechecker::test_prelude()` behind a `test-support` feature or in a shared dev-dependency module, that parses the bundled prelude modules and builds the `PreludeScope`. Install it in the unit tests and `tests/regressions.rs` wherever a checker is built directly, and in the checkers of `prism-cli/src/diagnostics.rs` tests and `compile_timing.rs`. Verify with `cargo test -p prism-typechecker`.

## 5. Typed tree, MIR and borrow checker (D5)

- [ ] 5.1 Add `Callee::External(DeclarationPath)` to `crates/prism-typed-tree/src/tree.rs` and `crates/prism-mir/src/mir.rs`, with `has_receiver() == false`. Dump it as `external <module>::<name>`, lower it in `build/call.rs` as a `Call` terminator, validate it in `validate.rs` like a function call, and let the borrow checker apply the callee's signature for elision as for `Callee::Function`. A named external used as a value lowers to an adapter closure calling it (`closures`). Verify with a `mir` fixture over a library test module (`// library:`) showing `external prism/test::prism_rt_print_int` in `mir.snap`.

## 6. Backend and runtime (D5, D10)

- [ ] 6.1 In `crates/prism-codegen/src/unit.rs`, add `UnitContext::external_function(symbol, parameters, result)`. It derives the Cranelift signature with `lower.rs::signature` and looks the symbol up in `RUNTIME_FUNCTIONS`. It compares the parameter and result machine types and the never-returns flag with the entry's `RuntimeType`s, and declares the import. A mismatch or a missing symbol becomes `CodegenError::ExternalMismatch` (`E6005`) at the declaration, naming the symbol and both signatures. In `lower.rs::call`, lower `Callee::External` as a direct `call` of that import, passing a result held in memory through an address first, as for `Callee::Function`. Externals get no slot and no symbol of their own in `symbols.rs`, and `supported.rs` accepts them. Verify with `lowering_tests.rs` tests over injected library modules (`prism_cli` test support) for "A direct call of an external", "An external called from library code", "A string result through an address", "An external with the wrong signature" and "An unknown external symbol". Add `E6005` to `UNFIXTURED_CODES` in `fixture_tests.rs`, naming these tests.
- [ ] 6.2 Add `crates/prism-runtime/src/process.rs` with `#[no_mangle] pub extern "C" fn prism_rt_exit(status: i64) -> !`. It flushes the output, calls the watch-mode stop hook when one is set (the hook `hot-reload` installs in `panic.rs::set_watch_mode`, extended to receive an exit status), and otherwise calls `std::process::exit((status & 255) as i32)` without the allocation check. Register it in `RUNTIME_FUNCTIONS` (`never_returns: true`). Make the watch session print `watch: main returned with status <status>; waiting for changes` for it, as for `main` returning. Verify with a runtime unit test in a child process (status 258 exits with 2), and with `standard-library` fixtures for "Exiting with a status", "Only the low bits of the status" and "A library module is imported" (`// run: 7`, `// run: 2`, `// run: 3`).
- [ ] 6.3 Make the file name a library unit records for runtime error locations its virtual path (`prism/collections.prism`). Verify with the `execution` fixture "An overflow inside a library helper" once task 8.1 exists.

## 7. Driver, cache and fixture runner (D3, D4, D13, D15)

- [ ] 7.1 In `crates/prism-cli/src/workspace.rs`, build the prelude scope once per check or run from the prelude nodes' ABIs and pass it to `typecheck_file` for workspace nodes. Ingest only direct imports for library nodes. Plan instances with prelude imports counted as direct imports (`instance-cache`'s summaries and `plan_instances`). Verify with the `compilation-and-abi` fixture "Instances of prelude generics" (with `instances-mir.snap`).
- [ ] 7.2 In `cache.rs` and the check-key computation, append the prelude modules' ABI hashes to a workspace file's key after its explicit imports. Key library modules by their own inputs. Verify with `cache_tests.rs` tests for "Library entries are shared between workspaces" (two temporary workspaces, one cache directory) and with the existing `keys_cover_every_input` test extended to the prelude.
- [ ] 7.3 Keep library files out of `CheckReport::checked_files` and `total_files`, and list them first in `--explain-cache` for `prism check` and `prism run` (`main.rs::print_cache_explanation` and the run explanation), shown by virtual path. Verify with fixtures and CLI tests for "A check report without library lines", "Unreached modules are not parsed" and "Explaining a warm check", and re-bless the existing `--explain-cache` snapshots, reading each against "Explaining Cache Use".
- [ ] 7.4 Add `library: Arc<Library>` to `CheckOptions` and to `run.rs`'s options (default `Library::bundled()`), threaded into graph construction. Do not expose it on the command line. Verify with the unit test `test_cached_report_matches_uncached_report` extended with an injected module.
- [ ] 7.5 In `crates/prism-cli/tests/fixture_tests.rs`, add the `// library: <dir>` header: the `.prism` files under `<case>/<dir>/prism/` become extra library modules for that case, checked in-process. Executable cases may not use it, and the runner rejects that combination. Document it in the file's header. Verify with the fixtures that tasks 2.5, 4.3, 4.6 and 5.1 name, including `E4029` cases for the catalog check.

## 8. Library content and its own checks (D8, D11, D13)

- [ ] 8.1 Add the helpers of "Higher-Order Helpers", "Visiting Helpers", "Testing Helpers", "Mapping and Folding Helpers" and "Filtering Consumes the List" to `collections.prism`, with `import core` for what they use. Write `sumOf` as `total = total + selector(x)`. Write `filter` by reversing the consumed list and moving `removeLast()` results into the new one (`collections` D17). Verify with `collections` fixtures for each scenario, executable ones under `--check-allocations`, and with the `standard-library` fixture "Helpers need no import".
- [ ] 8.2 Check every library module against the rules: format with `cargo run -q -p prism-cli -- fmt crates/prism-library/prism` and read the diff, and make sure each declaration has a comment saying what it does.
- [ ] 8.3 Add `crates/prism-cli/tests/library_tests.rs`. It builds `DependencyGraph::from_library(Library::bundled())`, runs Phase 1 and Phase 2 over it and fails on any diagnostic. It formats each module with `prism_formatter` and compares the text. It compiles every module's unit with the backend, so every external meets task 6.1's check. It compares the exported names of the prelude modules for overlaps, and compares `MODULES` with the `.prism` files of `crates/prism-library/prism/`. Verify by running it, and by breaking each rule once locally (a misformatted line, an extra file, a wrong external) to see it fail.

## 9. Integration

- [ ] 9.1 Edit the diagnostics catalog (`openspec/specs/diagnostics/spec.md`):
  - add `E1008` `ReservedPackage`, `E3002` `LibraryOnlyDeclaration`, `E4029` `InvalidLibraryDeclaration` and `E6005` `ExternalMismatch`;
  - widen `E4026` to print arguments;
  - make `E4010` name members of the library instead of the hand-written list;
  - make `E4013` mention prelude intrinsics named as values;
  - make `E2001` mention a function without a body.

  Verify with `fixture_codes_match_the_diagnostics_catalog`.
- [ ] 9.2 Use the library in `examples/run/`: a struct printed through its `toString()` and one helper (`map` or `forEach`). Run `prism check` on every file under `examples/`, `prism fmt --check examples/` and `prism run --check-allocations examples/run/main.prism`, which must print what its source computes.
- [ ] 9.3 Add a shape to `crates/prism-cli/tests/compile_timing.rs`: N workspace files, each calling `println`, `listOf` and two helpers. Run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` and check that the per-file cost stays flat as N grows (D4). Re-run the startup benchmark of `run_timing.rs` cold and warm, record both medians in design.md (D3), and keep them under the 15 ms target.
- [ ] 9.4 Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (no new warnings) and `cargo test --workspace`.
- [ ] 9.5 In `docs/architecture.md` §5, remove the requirement "Bundled Standard Library and Compose Prelude", except its scenario "Compose primitives need no import", which becomes a sentence in the `/prism/compose` plan for `compose-transform` (D14). Update the layer table's "Bundled library" row to say that the mechanism exists and the compose modules are added by their changes.

## Workflow follow-up

- At archive, add to `ROADMAP.md` the follow-ups of the proposal's "Not in scope". Tick "`print`/`println` of any type with a text form" and the exit-status item. Mark "Printing structs, enums and nullables" as partly done (structs and enums with a `toString()`).
- At archive, promote D1 to D7 and D9 into a "Design Rationale" section of the new `standard-library` spec, and D2 into `package-resolution`'s.
