# Design: explicit-library

## Context

See `proposal.md` for motivation. This change implements the parts of agreed item **A4** of the post–Milestone 2 review (`docs/milestone-2-review-notes.md`, "Agreed changes") that need no new language feature: "explicit imports everywhere, no prelude, to be revisited after seeing the result", and the agreed follow-ups, "built-in type names (`Int`, `Double`, `Bool`, `Char`, `String`, `Unit`, `Nothing`, `List`, `Heap`, `Range`) and the members of built-in types stay in scope everywhere; every function (`println`, `listOf`, `map`, `remember`, `mutableStateOf`, `Column`, ...) needs an explicit import. Location (a): `/prism/...` keeps resolving to the library from any workspace, but its sources move to a visible top-level directory (e.g. `library/prism/...`) as ordinary compilation units, with no prelude and fewer intrinsics." It is item 6 of "After Milestone 2: the review round" in `ROADMAP.md`. "Fewer intrinsics" is item 10 (`library-in-prism`), which needs `operator-conventions`, `drop-and-clone-hooks` and `vararg` first; this change keeps every intrinsic as it is. Decisions the user took are marked **(user direction 2026-10-20)**; every other decision the user might take differently is **(autonomous)** and logged in `docs/decisions-for-review.md`. Compile speed is a primary criterion (AGENTS.md §5): each decision states its compile-time cost.

**Baseline.** Three packets are proposed and not archived, and land before this one: `path-captures` (item 3, being implemented), `owned-state` (item 4) and `strong-skipping` (item 5). The ADDED requirements "The State Declarations of the Compose Runtime Module" (standard-library) and "The Compose Runtime Module" (compose) are written on `owned-state`'s versions of the blocks they replace (`State<T>` and `MutableState<T>` owning, `WeakReference<T>`, `weak`, `get`, `attached`). The first task re-bases every block on the main specs once those packets are archived. Their fixtures and examples need imports too; the migration (D10) handles whatever exists at apply time.

**Baseline update (apply, 2026-10-20).** `path-captures`, `owned-state` (fd397f6) and `strong-skipping` (c935cc8) are archived. Every MODIFIED block was compared with the main text: each keeps all the scenarios of its main requirement, and differs only by the prelude wording this change replaces. The ADDED blocks that replace REMOVED ones ("The State Declarations of the Compose Runtime Module", "The Compose Runtime Module", "The Collections Module Declares Lists and Their Helpers", "Extensions Resolve Through the File and Its Imports", "Member Modules Are Inputs of Every Workspace File") match the main texts `owned-state` and `strong-skipping` left, scenario for scenario but the one each REMOVED reason names; `/prism/compose/runtime` declares exactly the 21 entries `library_tests.rs::signatures_of_the_compose_module` lists. The main scenarios "Signatures of the compose module" (standard-library) and "The ABI of the compose module" (compilation-and-abi) still showed `escaping` composition contents and `<T : Copy> remember`, which `owned-state` changed to `attached` and `<T>`: the MODIFIED "The Compose Module" now shows the library's signatures, and a MODIFIED "Intrinsic Structs in the ABI" corrects the other (no behavior change; the test already checks the library's text). No main requirement outside this packet mentions the prelude; the Design Rationale bullets and the diagnostics catalog rows citing it are archive steps (Workflow follow-up). Every name the tasks use exists (`bind_import` is a closure of `workspace.rs`'s Phase 1, not a function). `openspec validate explicit-library --strict` is clean.

Current state (main specs and code):

- **The bundled library** (`standard-library` D1, D3): `crates/prism-library/prism/**.prism`, nine modules, about 1,230 lines, embedded by `include_str!` in `MODULES` (`crates/prism-library/src/lib.rs`), checked against the directory by `library_tests.rs::the_embedded_modules_are_the_library_directory`. Library nodes are keyed by virtual paths (`prism/core.prism`), never read from disk, root-discovered or boundary-checked, and resolve their imports within the library only (`library_node` in `crates/prism-resolver/src/graph.rs`).
- **The prelude** (`standard-library` D4): `PRELUDE` lists `prism/core`, `prism/collections` and `prism/compose/runtime`. The resolver gives every workspace node `prelude_imports`; `workspace.rs` binds Phase 1 type names to the prelude after the direct imports (`bind_import`), builds one `PreludeScope` per graph from the prelude ABIs (`PreludeScope::from_abis`) and installs it in every workspace file's checker (`TypeChecker::with_prelude`), which consults it last for names (`ScopeStack::lookup_prelude`), extensions, members (`TypeChecker::member`), intrinsic functions and externals, and inserts the prelude modules into `direct_imports`, so `E4014` never fires for their types. Check keys append the prelude modules' ABI hashes (`SourceNode::imports_with_prelude`). Library files see no prelude.
- **Members** (`standard-library` D7): an intrinsic extension is a member of its receiver type "wherever its module is visible": before every extension, after fields and the built-in `clone()` and `unbox()`, and a workspace extension of its name on its receiver is `E4010`. All intrinsic extensions of `/prism/core` and `/prism/collections` have built-in receivers; those of `/prism/compose/runtime` have its intrinsic structs as receivers; `/prism/compose/ui` declares `Int.dp` and `Int.sp`.
- **Unknown type names stay opaque** (`generics` D12): `fun f(s: Foo) {}` typechecks with no declaration of `Foo`; a use of a member or constructor reports the error. So a missing import of a library type surfaces where it is used, never at the annotation.
- **Fixtures** (`crates/prism-cli/tests/fixtures/`): about 2,200 cases and 2,400 `.prism` files, about 530 of which print, with 464 `stdout.snap`, 150 `stderr.snap` and about 260 MIR, typed-tree and layout snapshots, most with line and column positions. Thirteen cases inject library test modules (`// library:`). About 25 Rust test files build checkers with `test_prelude()`.

## Goals / Non-Goals

**Goals:**
- Every name a file uses is declared in the file or by a module it imports, except the built-in types and their members, and that rule fits in one sentence.
- The library is visible and readable at the top of the repository as ordinary `.prism` files, while the compiler stays one self-contained, hermetic executable.
- A missing import costs the user one look at the help line.
- No new compile-time cost on the success path; less implicit work per file.

**Non-Goals:**
- Moving intrinsics into Prism, or changing what any library declaration means (`library-in-prism`).
- Renaming, splitting or merging library modules (D13).
- Re-exports, wildcard or selective imports, import aliases.
- Checking `library/` in place with `prism check`, or loading the library from a directory at run time (D5, D6).

## Decisions

### D1. No prelude: every library name is imported (user direction 2026-10-20)

No library module is an implicit import. A file sees a library function, extension, struct, enum or top-level value only through an import of its module, which behaves exactly as an import of a workspace module (D4). `PRELUDE`, `SourceNode::prelude_imports`, the prelude binder of Phase 1, `PreludeScope`'s names, extensions and intrinsic functions, and `ScopeStack`'s prelude layer go away. The user asked to revisit the decision after seeing the result: the archive step records, for the examples, how many imports each file gained (D10), as the evidence for that review.

**Result (apply, for the user's review).** The migration gave 8 of the 15 files under `examples/` 14 library imports in all: `run/main.prism` 3 (`collections`, `compose/runtime`, `core`), `run/compose/counter.prism` and `run/compose/screen.prism` 2 each (`compose/runtime`, `core`), `apps/todo/model.prism` 2 (`collections`, `compose/runtime`), `apps/todo/screen.prism` 2 (`collections`, `compose/runtime`, now 5 library imports with the toolkit's three), `apps/todo/large.prism`, `apps/counter/main.prism` and `demo/ui/ui.prism` 1 each (`compose/runtime`; the UI files now import 4 or 5 library modules). The seven files that compute without printing or composing gained none. Among the fixtures, 635 of about 2,300 cases gained imports, most of them `import /prism/core` for `println`.

**Compile-time cost.** Lower than today. A lookup that misses the file and its imports fails at once instead of probing a shared layer; Phase 1 binds type names against direct imports only. A file importing library modules ingests their ABIs as it ingests any import's, linear in what it imports: `/prism/core` is five functions, `/prism/collections` about thirty declarations. The prelude benchmark measured 28 µs per file when each checker ingested all three prelude modules and 13 µs with a shared scope; a file that imports only `/prism/core` now pays a few microseconds, and one that imports nothing pays only D2's pointer copy. `compile_timing.rs` gains the shape `library-imports` (D14).

Rejected:
- **Keeping a prelude of functions only** (`print`, `println`, `listOf`): the user asked for none.
- **A `prism/prelude` module that a file imports once**: Prism has no re-exports, and one import naming everything is a prelude by another name.

### D2. Built-in types and their members need no import, through the member modules (user direction 2026-10-20 for the rule; the member modules autonomous)

The rule as users read it: **types and their members are everywhere; everything else of the library is imported.** The built-in types are the ten the follow-up lists. A built-in type's members are:

- the compiler's built-in members and syntax, which never had declarations: `clone()` of a `Clone` type, `unbox()` of a `Heap<T>`, `Heap(value)`, indexing, operators, `==`, `for` over a `List` or a `Range`, templates;
- the intrinsic extensions declared on it by its **member module**: `/prism/core` for `Int`, `Double`, `Bool`, `Char` and `String` (`toString()`, `length`), `/prism/collections` for `List` (`size`, `indices`, `isEmpty`, `isNotEmpty`, `contains`, `indexOf`, `add`, `insert`, `removeAt`, `removeLast`, `reverse`, `clear`).

`Unit`, `Nothing`, `Heap` and `Range` have no library members. Everything else those two modules declare is imported: `print`, `println`, `listOf` and the Prism-written helpers (`forEach`, `map`, `filter`, `any`, `count`, `fold`, `sumOf`, `joinToString`, ...), which are extensions, not members. So are `Int.dp` and `Int.sp`: they are intrinsic extensions of `Int`, but declared by `/prism/compose/ui`, not by `Int`'s member module, as Compose's `dp` is an extension imported from `androidx.compose.ui.unit`.

Mechanism: the member modules stay implicit inputs of every workspace file, now contributing their intrinsic extensions only. `PreludeScope` becomes `MemberScope` (`crates/prism-typechecker/src/members.rs`): built once per graph from the two modules' ABIs, holding the `members` map and nothing else, shared through an `Arc` and installed with `TypeChecker::with_members`. `TypeChecker::member` consults the file's own and imported members, then the member scope. The resolver's `prelude_imports` becomes `member_imports` (`prism/core`, `prism/collections`), still counted by check keys (D14) and still parsed for every program, as today. The member modules are not inserted into `direct_imports`, so their other names stay out of scope.

**Compile-time cost.** One `Arc` copy per workspace file and one hash lookup per member access that the file's own and imported members miss, as today; the scope shrinks to about twenty entries. The two member modules are parsed, extracted and cached once per process, as two of today's three prelude modules are.

Rejected:
- **Members typed straight from the intrinsic catalog, loading no module** (a file without imports would depend on no library module at all): purer, but `library-in-prism` is to write members such as `toString()` in Prism, and a member with a body needs its module in the program. The member modules' declarations are where a built-in type's members are written; Rust's inherent methods of `str` likewise live in `core`, always linked.
- **Every intrinsic extension a member everywhere**: `Int.dp` would be a member of `Int` in every file, so no program could declare its own `dp`, and the UI module would have to be in every program.
- **Helpers as members** (`map`, `filter`): the user listed `map` among the names to import, and helpers are ordinary Prism code that a file's own extension may shadow.
- **A per-file member scope from imports only** (no implicit input): `s.length` would need `import /prism/core`, against the follow-up.

### D3. Members of library types come with their module (autonomous)

An intrinsic extension whose receiver is a library type (`Composition.setContent`, `recompose`, `dump`, `RecomposeScope.invalidate`, the state members) is a member of that type where its module is imported, as it is today wherever its module is visible. Only a file that receives such a value from another module without importing the type's module can miss it: `fun f(scope: RecomposeScope) { scope.invalidate() }` with no import names an opaque type (unknown type names stay opaque), and the call is `E4006` with help naming `import /prism/compose/runtime` (D8). Fields of library structs (`Modifier`, `Color`, ...) keep `E4014` when their module is not a direct import, as every struct's fields do.

**Compile-time cost.** None: the members arrive with the import's ABI, as today.

Rejected:
- **Members of every library type found everywhere** (Kotlin's members belong to their class wherever it is): every module declaring a type with members would become an implicit input of every file, a prelude for members of every library type.
- **`E4014` for a library type's member**: `E4014` says "import the type's module", which is right, but the type was never resolved (it is opaque without the import), so the checker cannot tell its module apart from a typo; the help of D8 says it whenever it is true.

### D4. Imported library names follow the rules of imports (autonomous, following from D1)

With no layer below the imports, the special shadowing rules of `standard-library` D4 go: a library name a file imports behaves as any imported name. The file's own top-level declaration of the name shadows it (`fun print(n: &Int)` beside `import /prism/core` makes `print(1)` the file's own, while `println` stays the library's), as "Top-level declarations shadow imports" decided for every import (user, `shadowing-and-import-paths`); a name or extension exported by two imports is `E4012` at use (a workspace module's `List<T>.forEach` beside `import /prism/collections`); the file's own extension shadows an imported one. A file that does not import a module may declare any of its names: `fun listOf(n: Int): Int` in a file without `import /prism/collections` is simply the file's own. Adding a function to a library module can now change only the files that import that module and import the same name from elsewhere, which the user sees as `E4012`; a file's own declaration keeps its meaning.

**Apply note.** The packet first said that a file's own declaration of an imported library name is `E4010`. The type checker has never done that for any import: the user decided in `shadowing-and-import-paths` that top-level declarations shadow imports (type-system Design Rationale), and the diagnostics catalog's wording of `E4010` ("a top-level declaration with the same name as an imported symbol") covers only a `val` beside an imported struct ("Only a Same-File Struct and Value Share a Name"). Following "the rules of imports" therefore means shadowing; the requirement "Library Names Follow the Rules of Imports" and its scenario "A file's own declaration of a library name" now say so, and the migration never meets the clash D10 planned for.

**Compile-time cost.** None beyond the import rules.

Rejected:
- **Library imports shadowed by declarations and other imports** (today's "found last" rule kept for imported library modules): two rules for one `import` statement.

### D5. The sources live in `library/`, embedded from there (user direction 2026-10-20 for the location; the embedding autonomous)

`git mv crates/prism-library/prism library/prism`: the module `prism/<name>` is the file `library/prism/<name>.prism` (`library/prism/compose/material3.prism`), each still a file of the package `prism` or below it, so `library/` is laid out exactly as a workspace whose root holds the package `prism`, and as the fixtures' injected `lib/prism/...` modules are. `crates/prism-library` stays the crate that embeds them: `MODULES` becomes `include_str!("../../../library/prism/core.prism")`, ..., and `library_tests.rs` compares it with the files under `library/prism/` (`library_directory()`), as it compares it with the crate's directory today. rustc records every `include_str!` file in the crate's dep-info wherever it lives, so editing a library file rebuilds `prism-library` and relinks `prism`, as today. Nothing is read from disk at run time; the compiler identity, which hashes the executable, keeps covering the library, so the build cache needs no change for the move.

**Compile-time cost.** None: the same bytes are embedded.

Rejected:
- **A path relative to the executable** (`<bin>/../library`): `cargo install` copies the binary alone, tests run binaries from `target/` that would need the path set, an install could pair one compiler with another library, and the compiler identity would have to hash a directory as well as the executable to keep the cache hermetic.
- **A build script generating `MODULES` from the directory**: a build step for a list a test already keeps exact.
- **Moving the crate itself into `library/`**: `library/` would mix a Rust crate with the Prism sources it is meant to show.
- **`PRISM_LIBRARY_DIR`** for library authors (rejected in `standard-library` D1 as well): a second input to every build. Library edits take a rebuild of one small crate, which `library-in-prism` can revisit.

`cargo publish` refuses `include_str!` paths outside the package, so a published `prism-library` would need a build script copying `library/` in; nothing is published, and the follow-up is recorded.

### D6. `prism` stays a reserved root package (autonomous)

`E1008` stays: a workspace file may not declare `package prism...`, and `/prism/<module>` resolves to the bundled module whatever the workspace holds. With the library visible as ordinary files, the reservation is what keeps "ordinary" from meaning "replaceable": `import /prism/core` means the same code in every workspace, a workspace `prism/` directory can never shadow it, and a diagnostic's `prism/core.prism` names one file. A copy of `library/` is therefore not the library: `prism check library/prism/core.prism` in the repository is `E1008`. The library is checked by `library_tests.rs`, which checks, formats and compiles the embedded modules.

**Compile-time cost.** None (one comparison of a package's first segment, as today).

Rejected:
- **Dropping the reservation**: either `/prism/...` keeps resolving to the library, and a workspace `package prism` file becomes unreachable and shown under the library's path, or a workspace can shadow the library, and an import means different code in different workspaces.
- **Exempting files under a directory named `library`, or files identical to an embedded module**: the first is a marker by name, against the zero-marker invariant; the second makes a file's meaning depend on its bytes matching the compiler's. Checking the library in place (`prism check --library <dir>`) is a follow-up for library authors.

### D7. Library files still see nothing implicitly (autonomous, unchanged from `standard-library` D4)

A library file imports every library module it uses, the member modules included: `s.length` in a library file needs `import core`. The library's dependency order stays explicit, the member modules never depend on themselves through an implicit input, and only workspace files get the one-sentence rule of D2.

**Compile-time cost.** None.

Rejected:
- **Member modules as implicit inputs of library files too**: `/prism/core` and `/prism/collections` would need an exception for themselves, and the library's own imports would stop showing what it depends on.

### D8. An unresolved library name gets help naming its import, from an index built on demand (autonomous)

When a bare name is `E4001`, or a member or extension call or read is `E4006`, `prism-cli` looks the name up in a **library index** before rendering, and adds one help line per library module that declares it publicly: `'println' is declared by the library module '/prism/core': add 'import /prism/core'`. The index maps each public name to its declaring modules and kind: a function, value, struct or enum by name; an extension by name and receiver type constructor name (`List`, `Int`, `RecomposeScope`), or "any" for a receiver that is a type parameter. An extension entry matches when the receiver's constructor, after references and `Heap`, has that name (an opaque type is matched by its written name) or the entry's receiver is a type parameter. A module the file already imports is never suggested (the name would have resolved), and no workspace module is ever searched.

The index is built from the program's `Library` (so injected test modules count) by parsing every module and walking its public top-level declarations: about 1,230 lines, well under a millisecond, once per process (`OnceLock` in `crates/prism-cli/src/library_index.rs`), on the first diagnostic that needs it. A program without such a diagnostic never builds it. `TypeError::UnresolvedSymbol` and `TypeError::UnknownField` gain `import_suggestions: Vec<String>`, which `prism-cli` fills (`suggest_library_imports`, called on each file's type errors in `workspace.rs` before they are returned) and `diagnostics.rs` renders as `help:` lines after any other help. The type checker stays free of library knowledge.

The help quotes names with `'...'`, as every other help line of the compiler does (`add 'import /prism/compose/ui' to this file` of `E4014`); the packet first showed backticks (apply note).

**Compile-time cost.** Zero on the success path; on an error path, one parse of the library per process and one hash lookup per unresolved name.

Rejected:
- **A static table in `prism-library`, checked by a test** (the cheap form suggested when the change was planned): exact only while maintained, and every library change, `library-in-prism`'s many, would edit two places; deriving the index from the sources costs nothing on the success path.
- **A table generated by a build script**: a build step for what a lazy walk does.
- **Ingesting every library ABI into each checker to resolve the name**: a prelude by another name, paid on the success path.
- **Suggesting workspace modules too**: it would read and parse the workspace on an error path; a follow-up for an LSP, which holds the workspace anyway.

### D9. `prism fmt` sorts imports (autonomous)

Explicit imports multiply (the todo screen will import about five library modules), and an order that depends on who added which import first churns diffs. The formatter prints a file's imports sorted by path text, a root-anchored path with its leading `/`, so root-anchored imports precede sibling ones and library imports fall among the root-anchored ones alphabetically (`/app/model`, `/prism/collections`, `/prism/core`, `widgets`); imports of one path keep their order (a stable sort; duplicates are the resolver's business). When a comment lies between the first and the last import, the imports stay as written, so comments never move relative to code ("Formatting Preserves Meaning"). Import order has no meaning: `E4012` and the Phase 1 binder treat a file's imports as a set, and only the check key hashes them in order, so a reorder re-checks the file once and its importers not at all (build-cache, "Reordering imports changes only the key"). The verifier (`crates/prism-formatter/src/verify.rs`) compares the imports as sorted lists.

**Compile-time cost.** None for compilation; formatting sorts `n` imports in `O(n log n)`.

Rejected:
- **Keeping the written order**: churn, and the migration's inserted imports would land wherever the script put them.
- **Groups separated by blank lines** (library, root-anchored, sibling, as Go and Rust tools group the standard library): more layout rules for little gain, since `/prism/` already sorts as a group.
- **Moving comments with the import below them**: comment attachment for imports, for a rare case.

### D10. One mechanical migration, driven by the compiler's own help (autonomous)

After D1, about 530 fixture cases, every example and many inline test sources stop compiling. They are migrated once by `openspec/changes/explicit-library/migrate_imports.py` (archived with the change as the record of how), run against the new compiler:

1. For each fixture case (the harness's entry rules: the header file or `// entry:`, the case directory as working directory), each example entry (`examples/run/main.prism`, the UI entries, `examples/demo/*`), and each edit version (`*.prism.v<n>`, checked in a temporary copy of its case with that version in place of its base file), run `prism check` and collect every help line naming `import /prism/...` with the file it is reported in.
2. Insert each named import into that file, after its package declaration and imports, keeping the import block sorted (D9), with one blank line after it when code follows. Repeat until no help is new (a fixpoint, at most four rounds, since one missing import can hide another's errors).
3. After inserting, re-check the file: when an inserted import brings a new `E4010` or `E4012`, revert that insertion and list the case (a file's own `print` beside an imported `/prism/core` shadows it, D4, so the clash this guards against is `E4012` with another import, or `E4010` of a `val` named like an imported struct). Skip and list, for hand review, every case whose expectation names `E4001`, `E4006`, `E4010` or `E4012` (it may test a missing or clashing name on purpose), every `// library:` case (thirteen; library test modules import what they use already), and every file the script could not settle.
4. Re-bless (`PRISM_BLESS=1`), then `migrate_imports.py --verify-snapshots` compares each changed snapshot with its previous version (`git show HEAD:<path>`): a difference is accepted only when every changed line differs by line numbers (`main.prism:3:5`, `@2:17`) shifted by the number of lines inserted above that position in that file. Anything else is listed and read by hand.

Inline sources of Rust tests that drive the CLI (`run_cli_tests.rs`, `cache_tests.rs`, `watch_session_tests.rs`, `cli_workspace_tests.rs`, `record_tests.rs`, the timing tests, the window tests) are fixed by hand, guided by the same help lines in their failures. In-crate unit tests keep their sources (D11). Fixtures of scenarios this change renames are renamed with them (`helpers-need-no-import` to `helpers-are-imported`, ...), and fixtures of the removed prelude scenarios become fixtures of their replacements.

**Compile-time cost.** None (a one-off).

Rejected:
- **A syntactic script** (identifiers intersected with the library's exports): it cannot tell a local `size` from a member or a file's own `print` from the library's, and would add imports that clash (`E4010`).
- **Rewriting snapshots' line numbers in the script**: it would have to know which snapshot positions belong to which file; re-blessing and verifying the shift checks the same thing with the compiler as the oracle.
- **Placing imports on the line of the package declaration** to avoid shifting lines: not a canonical layout, and `prism fmt` would undo it.

### D11. In-crate unit tests import the former prelude modules through a helper (autonomous)

About 25 Rust test files build a `TypeChecker` directly and call `with_prelude(test_prelude())`. `test_prelude()` becomes `test_library_imports()`: the parsed ABIs of `/prism/core`, `/prism/collections` and `/prism/compose/runtime`, built once per process, which `TypeChecker::with_library_imports` ingests as ordinary direct imports (and the member scope with them). Their inline sources stay as they are; they test the checker, MIR, borrow checker and backend, not name resolution, which the fixtures cover with real imports.

**Compile-time cost.** None (tests only).

Rejected:
- **Adding import lines to every inline source**: a direct checker resolves no imports, so the lines would need a resolver in each test harness, for hundreds of sources.

### D12. Scenarios omit library imports (autonomous)

About 800 scenarios across the specs run programs that print or build lists. A standard-library requirement ("Scenarios Omit Library Imports") says that a scenario without imports is read with the imports of the library modules declaring the names it uses, as its fixture declares them; scenarios about imports show them. The scenarios this change touches anyway show their imports.

**Compile-time cost.** None.

Rejected:
- **Rewriting every scenario with its imports**: a large, error-prone edit of every spec that changes no behavior and makes each scenario longer to read.

### D13. Module names and contents stay as they are (autonomous)

`print` and `println` stay in `/prism/core` (Kotlin has them in `kotlin.io`), `exitProcess` in `/prism/system`, the helpers in `/prism/collections`, and the compose and UI modules as `ui-toolkit` D21 split them after Compose's artifacts. Splitting `/prism/core` (an `/prism/io`) is `library-in-prism`'s decision, once it knows what the core holds; doing it here would migrate every program twice.

**Compile-time cost.** None.

### D14. Check keys and timing shapes (autonomous)

A workspace file's check key hashes its direct imports' ABI hashes in import order, then the member modules' (`/prism/core`, `/prism/collections`), replacing the prelude modules': a change of `/prism/compose/runtime`'s ABI no longer re-checks files that do not import it (moot today, since any library change changes the compiler identity, but true of the key's inputs). Instances of library generics are planned from imports, as any import's. `compile_timing.rs` replaces `prelude-calls` by `library-calls` (the same statements with the imports they need) and the benchmark `prelude_costs_a_lookup_per_file` by `library_imports_cost_per_file`: 256, 1,024 and 4,096 files each importing `/prism/core` and `/prism/collections`, asserting growth per file no worse than 1.2× between the first and last size under `PRISM_ASSERT_TIMING=1`, and reporting the time per file beside the member scope's.

**Compile-time cost.** As D1 and D2.

## Risks / Trade-offs

- **[A library type named without its import is opaque]** (found applying) Unknown type names stay opaque (`generics` D12), so `fun f(s: MutableState<Int>)` or a struct field of type `Composition` in a file that no longer sees `/prism/compose/runtime` typechecks, as an opaque type, and only a member, a construction or the backend reports anything (`dump-layout` shows "no layout yet"). The help only fires on such a use. Six fixtures naming runtime types in signatures alone passed for that reason after the migration (two layout snapshots and a composed MIR changed, which the snapshot check caught); they import the module by hand. → A follow-up could report an annotation naming a type that no import declares but a library module does (the library index already knows), as a help or an error; recorded in `ROADMAP.md` at archive.

- **[Verbose files]** A UI file imports up to five library modules and `/prism/core`. → The user asked to see the result before revisiting; the archive records the counts. Re-exports or an umbrella import are the obvious later options, both language changes.
- **[Migration noise]** About 900 snapshots shift line numbers. → D10's verification accepts only pure shifts, so a real behavior change cannot hide among them.
- **[Hidden semantic change in a fixture]** A case that relied on shadowing a prelude name (`fun print(n: &Int)`) keeps working, with or without the import, as D4 intends: its own declaration shadows the import. → The script still compares each case's codes with its expectation after inserting, and lists any case whose codes changed.
- **[Library edits need a rebuild]** Editing `library/prism/*.prism` takes effect only after `cargo build`. → As today; recorded as a follow-up (D5, D6).
- **[Packets landing in between]** `owned-state` and `strong-skipping` add fixtures, examples and library declarations before this change applies. → The migration runs on whatever exists; the first task re-bases the blocks written on `owned-state`'s versions.

## Migration Plan

1. Move the library to `library/` (one commit, no behavior change).
2. Add the library index and its help while the prelude still exists (testable with UI names, which are not in the prelude), and sorted imports in the formatter.
3. Remove the prelude, add the member scope, migrate every fixture, example and test, and rename fixtures, in one commit that passes `cargo test --workspace`.
4. Update `README.md`, `AGENTS.md`'s spec summaries and `docs/architecture.md`'s program samples.

Rollback: revert the commits; the move is a pure rename.

## Open Questions

- After seeing the result, does the user want a shorter form for common imports (re-exports, an umbrella module), or a prelude for a few names after all (the user's "to be revisited")?
- Should `prism check` gain a way to check a library directory in place for library authors (`prism check --library library/`), once `library-in-prism` makes library edits frequent?
