# Proposal: explicit-library

## Why

The post–Milestone 2 review agreed that the standard and compose libraries should be "as much as possible ... normal prism code that is imported like any other code" (item **A4** of `docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20). Two of its decisions need no new language feature and come first: the library's sources leave the compiler's crate for a visible top-level directory, and the prelude goes away, so every library function is imported explicitly ("explicit imports everywhere, no prelude, to be revisited after seeing the result"). The agreed follow-ups pin down the boundary: built-in type names (`Int`, `Double`, `Bool`, `Char`, `String`, `Unit`, `Nothing`, `List`, `Heap`, `Range`) and the members of built-in types stay in scope everywhere; every function (`println`, `listOf`, `map`, `remember`, `mutableStateOf`, `Column`, ...) needs an import; and location option (a) keeps `/prism/...` resolving to the library from any workspace while its sources move to `library/prism/...`. This is item 6 of "After Milestone 2: the review round" in `ROADMAP.md`. Today a reader cannot see where `println` or `remember` comes from, a file's meaning depends on an invisible layer, and the library hides in `crates/prism-library/prism/`.

## What Changes

- **The library lives in `library/`.** `crates/prism-library/prism/**` moves to the top-level `library/prism/**` (`library/prism/core.prism`, `library/prism/compose/ui.prism`, ...). The executable still embeds every module with `include_str!` from that directory, reads nothing from disk, and `/prism/...` resolves to the embedded modules from any workspace, test fixtures included. The compiler identity, which hashes the executable, keeps covering the library.
- **No prelude** (user direction). `/prism/core`, `/prism/collections` and `/prism/compose/runtime` are no longer implicit imports: `print`, `println`, `listOf`, the list helpers (`map`, `filter`, `forEach`, `joinToString`, ...), `remember`, `key`, `composition`, `mutableStateOf`, `State`, `MutableState` and every UI name are used after an import of their module, like a workspace module's names. **BREAKING**: every program that prints or builds a list gains `import /prism/core` or `import /prism/collections`.
- **Built-in types and their members need no import** (user direction). The rule, in one line: *types and their members are everywhere; everything else of the library is imported*. A built-in type's members are the compiler's (`clone()`, `unbox()`, indexing, operators, `for`) and the intrinsic extensions its **member module** declares on it: `toString()` of the scalars and `String.length` in `/prism/core`; `size`, `indices`, `add`, `removeAt`, `contains`, ... of `List` in `/prism/collections`. The helpers written in Prism (`map`, `filter`, ...) are extensions, so they are imported; so are `Int.dp` and `Int.sp`, extensions of `Int` declared by `/prism/compose/ui`. Members of library types (`Composition.recompose`, `MutableState.value`) are found where their module is imported.
- **Imported library names follow the rules of imports.** A file's own declaration of a name it imports shadows the import, as for any import; two imports exporting a name or extension are `E4012` at use; a file that does not import a library module may declare its names freely. The "found last" shadowing layer disappears.
- **Diagnostics suggest the import.** An unresolved name (`E4001`) or member (`E4006`) that a library module declares gets `help: 'println' is declared by the library module '/prism/core': add 'import /prism/core'`, from an index of the library's public declarations built on the first such diagnostic only.
- **`prism` stays a reserved root package** (`E1008`), so `/prism/core` means the same code in every workspace and no workspace can shadow it.
- **`prism fmt` sorts imports** by path (root-anchored before sibling ones), keeping the written order when comments lie among them.
- **One mechanical migration.** A script driven by the new help adds the imports every fixture, edit version and example needs, run once to a fixpoint and reviewed; snapshots are re-blessed and checked to differ only by shifted line numbers. In-crate unit tests that build a checker directly import the three former prelude modules through a test helper instead.
- **Not in scope:** moving intrinsics into Prism (`library-in-prism`, item 10, after `operator-conventions`, `drop-and-clone-hooks` and `vararg`); renaming or splitting library modules (`print` stays in `/prism/core`); re-exports; checking `library/` in place with `prism check`; a library-development override such as `PRISM_LIBRARY_DIR`.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `standard-library`: sources in `library/`; no prelude; library names imported and following the rules of imports; built-in types and their members everywhere, through the member modules; intrinsic extensions as members where in scope; the module requirements' scenarios with imports; a convention that scenarios omit library imports.
- `package-resolution`: the reserved root package kept, with a copy of the library's sources not being the library.
- `extension-functions`: extension resolution through the file and its imports only, context extensions without a prelude layer.
- `compilation-and-abi`: member modules, not prelude modules, as implicit inputs of workspace files.
- `build-cache`: check keys covering the member modules; import order changing only the key.
- `compose`: `/prism/compose/runtime` as an imported module; the composer protocol module without the prelude wording.
- `collections`: helpers imported like any extension.
- `ui`: toolkit modules without the prelude wording; their unresolved names suggest the import.
- `diagnostics`: help naming the import of an unresolved library name.
- `formatting`: sorted imports.

## Impact

- `library/prism/**` (moved from `crates/prism-library/prism/**`), `crates/prism-library/src/lib.rs` (`MODULES` paths, `PRELUDE` replaced by `MEMBER_MODULES`).
- `crates/prism-resolver/src/graph.rs` (`prelude_imports` becomes `member_imports`, the member modules only).
- `crates/prism-typechecker`: `prelude.rs` becomes `members.rs` (`MemberScope`: the member modules' intrinsic extensions only), `scope.rs` (no prelude layer), `checker.rs` (`with_prelude`, member, intrinsic and extension lookups), `test_prelude()` replaced by `test_library_imports()` in every crate's tests.
- `crates/prism-cli`: `workspace.rs` (no prelude binder or scope, the member scope), `cache.rs` (check keys), `diagnostics.rs` and a new `library_index.rs` (import help), `tests/library_tests.rs`, `tests/fixture_tests.rs` (fixtures renamed after their scenarios), `tests/compile_timing.rs` (the prelude benchmark replaced by `library-imports`).
- `crates/prism-formatter/src/printer/declarations.rs` and `verify.rs` (sorted imports).
- About 2,400 fixture `.prism` files and edit versions under `crates/prism-cli/tests/fixtures/` (about 530 print), their `stdout.snap`/`stderr.snap`/MIR and typed-tree snapshots, inline sources of CLI-driven Rust tests, every file under `examples/`, and the packets landing before this one (`path-captures`, `owned-state`, `strong-skipping`), whatever exists at apply time.
- `README.md`, `AGENTS.md` (the spec summaries mentioning the prelude), `docs/architecture.md` where it shows programs.
