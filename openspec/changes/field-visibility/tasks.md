# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/`, one case per scenario, or one per form of a scenario that has several (`<scenario-slug>--<form>`), as the header of `fixture_tests.rs` describes. Each case runs real `.prism` source; multi-file scenarios use the fixture workspace layout the existing `type-system` cross-module cases use. Executable cases hold `stdout.snap` and `stderr.snap` (written with `PRISM_BLESS=1`) and run under `--check-allocations`. Read each snapshot against its scenario's THEN before committing it.

Commit one group at a time, each passing `cargo test --workspace` ("Implement field-visibility: <group>"). Another agent may be working in the same checkout, so stage explicit paths only.

## 1. Reconcile and baseline

- [x] 1.1 Check that `drop-and-clone-hooks`, `vararg` and `core-library-in-prism` are archived, and stop if one is not.
  - Re-read the main `syntax`, `type-system`, `extension-functions`, `patterns`, `compilation-and-abi`, `formatting`, `ui` and `diagnostics` specs.
  - Check that no requirement this packet ADDS exists by then. Check that "Only Visible Fields Count" still matches the field rules of `extension-functions` as `core-library-in-prism` left them, its extension property rules included.
  - Check the names the tasks use: `StructField` (`crates/prism-ast/src/decl.rs`), the struct field parsing in `crates/prism-parser/src/parser.rs`, `ExportedSymbol::Struct` (`crates/prism-abi/src/signature.rs`), `register_extension_decl`, `infer_member_access` and the construction and place paths of `crates/prism-typechecker/src/checker.rs`, and `check_take_apart_source` and the struct pattern checks of `checker/patterns.rs`.
  - Get `openspec validate field-visibility --strict` clean.
  - Record the reconciliation as a "Baseline update" paragraph in `design.md`.
- [x] 1.2 On an idle machine and a release build, record in `design.md` the `struct-fields` stages of `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture`, as the baseline that task 5.1 compares with.

## 2. Syntax and formatting (D1, D8)

- [x] 2.1 AST: `StructField` gains `visibility: Option<Visibility>`, `None` when unmarked. Update every construction and match on it: the parser, formatter, ABI extraction, type checker and test helpers. Verify with `cargo build --workspace`.
- [x] 2.2 Parser: accept one of `private`, `package` and `public` before `val`/`var` of a field, in block and shorthand structs. Reject with `E2001`: two modifiers, a repeated one, a modifier not followed by `val` or `var`, and a modifier on an enum payload, with the message of "Misplaced field modifiers rejected". Add fixtures under `syntax/` for "Field modifiers in both struct forms" and "Misplaced field modifiers rejected". Verify with `cargo test -p prism-parser -p prism-cli --test fixture_tests`.
- [x] 2.3 Formatter: print the modifier as written, before `val`/`var`, in both struct forms. `verify.rs` compares the new field. Add a fixture or `fmt_cli_tests.rs` case for "Field modifiers formatted", and an idempotency check over the new `syntax/` fixtures. Verify with `cargo test -p prism-formatter -p prism-cli --test fmt_cli_tests`.

## 3. ABI (D6)

- [x] 3.1 `ExportedSymbol::Struct`'s fields carry their visibility, with an explicit `public` recorded as unmarked. The ABI hash covers it, and `prism abi` prints `private val id: Int` and `package val bank: String`. Update the ABI encoding of the build cache if it serializes fields. `AbiTypeDefinition` is unchanged. Add a fixture under `compilation-and-abi/` for "The ABI shows field modifiers", and a `cache_tests.rs` case for "Hiding a field re-checks importers", which needs two runs. Verify with `cargo test -p prism-abi -p prism-cli --test cache_tests`.
  - Done (apply): fixtures run `prism check`, not `prism abi`, so "The ABI shows field modifiers" is a `cache_tests.rs` case beside the other `prism abi` cases. The build cache stores no ABI symbol, so it had no encoding to update. The visibility is a list `field_visibilities` beside `fields`, which `AbiTypeMembers::Fields` shares unchanged; the hash folds it into the mutability byte, so an unmarked field hashes as before. The `E4007` half of "Hiding a field re-checks importers" is asserted with task 4.2.

## 4. Type checker (D2 to D5, D9)

- [ ] 4.1 Field records in the checker (local and imported) hold their effective visibility. Add one helper, `field_visible(struct, field, file)`, comparing it with the current file and package.
- [ ] 4.2 Accesses (D3):
  - `infer_member_access`, assignments, compound assignments, borrows, places and path captures report `E4007` for a hidden field, naming the field, its visibility and the declaring module;
  - bare names in extension bodies and lambdas with a receiver skip hidden fields, and report `E4007` only when nothing else resolves the name;
  - a hidden field never yields `E4006`.

  Add fixtures under `type-system/` for "Unmarked fields keep their meaning", "A package field is visible in its package", "Reading and writing a private field from another file", "A private field in its own file" and "Whole values with hidden fields are used freely". Verify with `cargo test -p prism-typechecker -p prism-cli --test fixture_tests`.
- [ ] 4.3 Construction (D2): positional and named constructions check every field's visibility, reporting `E4007` for the first hidden one. Add fixtures for "A handle cannot be forged".
- [ ] 4.4 Resolution (D5): the call precedence, the property reads of `core-library-in-prism`, bare names, and the `E4010` checks of `register_extension_decl` and of extension property declarations count only fields visible in the file. Add fixtures under `extension-functions/` for "An extension named like a hidden field", "An extension named like a visible field is still rejected" and "A hidden field does not shadow an outer name".
- [ ] 4.5 Patterns (D4): positional struct patterns are `E4007` when any field is hidden; named sub-patterns and destructuring entries naming a hidden field are `E4007`; leaving hidden fields out stays valid, and exhaustiveness is unchanged. Add fixtures under `patterns/` for "Patterns over a struct with a private field" and "Destructuring a hidden field rejected". Verify with `cargo test --workspace`.
- [ ] 4.6 Diagnostics catalog (`openspec/specs/diagnostics/spec.md`): widen the `E4007` row to hidden fields (reads, writes, borrows, path captures, constructions, patterns and destructuring entries), and the `E2001` row to misplaced field modifiers.

## 5. Library, timing and verification (D7, D10)

- [ ] 5.1 `library/prism/compose/ui.prism`: mark `Modifier`'s fields `count`, `first` and `second` and `ModifierElements`' fields `e0` to `e7` `private`. Check that no other library module, example or fixture reads them. Add fixtures under `ui/` for "A chain cannot be built or read by hand" and "Chains built through modifier functions still compare". Run `cargo test -p prism-cli --test library_tests`.
- [ ] 5.2 Add the `field-visibility` shape to `crates/prism-cli/tests/compile_timing.rs` (D10). Run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` on an idle machine, and record its stages and the `struct-fields` comparison with task 1.2 in `design.md`.
- [ ] 5.3 Run the apply checks of `openspec/config.yaml`:
  - `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`, reading the `test result:` lines;
  - `prism check` on every file under `examples/`, and `prism fmt --check examples/`;
  - `prism run --check-allocations examples/run/main.prism`, and each UI entry with `--headless`.
- [ ] 5.4 Update `ROADMAP.md`: tick item 10b, and mark done the `extension-methods` item "Field visibility (`private var balance`)". Mark the element-block half of the `ui-toolkit` follow-up "Field visibility, so that `ModifierElement`'s payloads and the element blocks are not readable or constructible by programs" done, and leave the payload half open. Add the follow-ups of the proposal's "Not in scope" list as unchecked items naming this change.
