# Design: standard-library

## Context

See `proposal.md` for motivation. This is item 26 of "Then: the road to Milestone 2" in `ROADMAP.md` and the requirement "Bundled Standard Library and Compose Prelude" of `docs/architecture.md` §5, whose decision D2 (`compose-architecture`) chose Prism source shipped with the compiler over type-checker tables. `collections` shaped its built-ins to move here: its D1 made `List<T>` a compiler type whose members are `Extension` records "exactly what `intrinsic` extension declarations in a bundled file would build" (its D6), and its D17 left `forEach`, `map` and `filter` to this library. Three follow-ups wait for it too: "`print`/`println` of any type with a text form" (`string-templates`, D6), "Printing structs, enums and nullables" (`codegen-core`, D16) and "An exit-status API (`exitProcess`...)" (`codegen-core`, D13). The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19), so each decision states its compile-time cost.

**Baseline.** This change was proposed while `monomorphization` was being implemented and `instance-cache`, `closures`, `escaping-closures`, `collections` and `hot-reload` were proposed but not implemented. All six land before it, and it needs them. The library's helpers are generic code that runs as instances. Its entries are cached through `instance-cache`'s check, record and code entries, keyed by ABI hashes with a compiler identity that hashes the executable. The helpers take closures, and `/prism/collections` declares `collections`' members. Watch sessions must leave library files alone. When it is applied, these exist:

- `collections`: `Type::List`, `BuiltinFunction::ListOf` and one variant per list member, the member table of `Extension` records with `builtin: Option<BuiltinFunction>` consulted in `infer_method_call` and `build_member_access` (its D6), `size` and `indices` read like `length`, and `listOf` found last like `print`;
- `instance-cache`: check keys over the direct imports' ABI hashes (the interface hash is merged into the ABI hash), records and summaries, program-wide instance planning (`plan_instances`), per-unit code entries, `prism run` using the cache by default, and `compiler_identity()` covering the executable's content;
- `closures` and `escaping-closures`: function values, adapter closures for named functions, and the `escaping` modifier;
- `hot-reload`: a `SourceStore` feeding `DependencyGraph::from_entry_with`, and a watch session that rebuilds the graph on change.

The MODIFIED requirements were written against the texts those packets leave: `build-cache` "Cache Keys Cover Exactly the Compilation Inputs" and "Explaining Cache Use" from `instance-cache`, `code-generation` "Calls Go Through the Function Table" from `hot-reload`, `syntax` "Contextual Keyword Identifier Permissibility" from `escaping-closures`, and `collections` from `collections`. Compare each with the main specs at apply time.

What exists today, apart from those:

- **Resolver** (`crates/prism-resolver`). `DependencyGraph::from_entries` reads each file from disk, discovers its root (`discover_root`), checks root consistency (`E1004`) and resolves imports with `resolve_import`, which maps `/a/b` to `<root>/a/b.prism` and a sibling `b` to `<dir>/b.prism`. `SourceNode` holds the path (the map key), package, source, AST and `direct_imports`. `workspace.rs::assert_workspace_boundary` canonicalizes every node path (`E1005`).
- **Phase 1** (`crates/prism-abi`). `FileAbi::extract_with_definitions` reads exported declarations from the AST. `ExportedSymbol` has `Function`, `Struct`, `Enum` and `Var`. The parser already accepts a `fun` without a body (`FunctionDecl::body: Option<FunctionBody>`, "forward declaration / interface"), which nothing specifies or uses.
- **Type checker** (`crates/prism-typechecker/src/checker.rs`). `print`/`println` are `output_builtin`, consulted in `infer_call_expr` when no declaration or import of the name exists, and checked by `infer_output_call` and `check_printed_argument`, which accepts scalars and strings and reports anything else as `E4004`. Template parts have the text-form rule (`check_template_part`, `template_to_string_problem`, `E4026`). `toString()` of scalars and `clone()` are special cases in `infer_method_call`, `length` in `build_member_access`. Imports are a `ScopeStack` import layer (`define_import`, `lookup_import`), and extensions are `own_extensions` and `imported_extensions`, found by `find_extension`. `E4010` for uncallable extensions is a hand-written list.
- **Backend** (`crates/prism-codegen`). Prism calls load their slot (`lower.rs::call`, `slot_target`). Runtime calls are direct imports declared from `prism_runtime::RUNTIME_FUNCTIONS`, each with a `RuntimeType` signature (`unit.rs::runtime_function`). The runtime has `prism_rt_int_to_string(destination, value)` and friends, which already take a result held in memory through an address passed first, the convention of Prism functions.
- **CLI** (`crates/prism-cli`). `workspace.rs` drives Phase 1 (`extract_abis`) and Phase 2 (`elaborate_file`, `typecheck_file`, which builds a `TypeChecker` and calls `ingest_file_abi` per direct import). Error rendering re-reads files with `fs::read_to_string`.

## Goals / Non-Goals

**Goals:**
- One mechanism, Prism declarations, for every name a program uses without declaring it. The compose engine then adds modules, not type-checker tables.
- A prelude that costs a file a constant amount, whatever the library's size.
- An FFI form the compose and UI runtimes can use as is, with its ABI checked by the compiler rather than by convention.
- Library code that is checked, formatted, cached, instantiated and located in errors like user code.
- No change to what today's programs mean, except the silent acceptance of body-less functions and `print` accepting more types.

**Non-Goals:**
- `/prism/compose` and `/prism/ui` (D14), intrinsic types (`State<T>`), and extension properties or `external` for user code.
- Moving `Heap`, `Range`, `clone()`, indexing or the built-in types into the library (D8).
- Math, `error`/`require`, standard input, program arguments (D8, D10).
- Precompiled or cached library ABIs, and skipping unused library units (D3).

## Decisions

### D1. The library is embedded in the executable, from a `prism-library` crate (autonomous)

A new crate `crates/prism-library` holds the sources under `crates/prism-library/prism/` (`core.prism`, `collections.prism`, `system.prism`, all `package prism`) and a static list in `src/lib.rs`: `LibraryModule { path: "prism/core", source: include_str!("../prism/core.prism") }` for each, plus `PRELUDE: &[&str] = &["prism/core", "prism/collections"]`. A `Library` value wraps a module list and a prelude list. `Library::bundled()` is the embedded one, and `Library::with_modules(extra)` adds modules, for tests only (D13). The crate depends on nothing. No build script: a test (D13) checks that the static list matches the directory.

Embedding makes the compiler one self-contained file, so an installation cannot be half-updated or point at the wrong library, and reading the library costs no file system access. The library is covered by the compiler identity for free, since `instance-cache`'s identity hashes the executable that holds the sources. The sources live inside the crate so that `include_str!` paths stay within the package. Changing a library file rebuilds only `prism-library` and relinks `prism`.

Compile-time cost: none at run time beyond reading static memory. The cost at Rust build time is one small crate.

Rejected:
- A `lib/` directory next to the binary, as Zig and Go ship theirs: an install-layout dependency, and a library that can drift from the compiler that checks it.
- Materializing the sources into the cache directory on first use: a writable disk on every run, and two copies to keep in step.
- A `PRISM_LIBRARY_DIR` override for library development: a second input to every build, needed only by library authors, who rebuild one crate. It is a follow-up if the library grows.
- A build script generating the module list: slower builds for a list of three entries.

### D2. `prism` is a reserved root package, resolved to the library (autonomous)

- **Imports.** In `resolve_import`, a root-anchored import of two or more segments whose first segment is `prism` resolves to the library module `prism/<rest>`, never to `<root>/prism/...`. An unknown module is `E1002` naming the library. `import /prism`, a single segment, stays a workspace import, so a root file named `prism.prism` keeps working.
- **Reserved package.** A workspace file whose package's first segment is `prism` is the new `E1008` (`ReservedPackage`), raised in `from_entries` after parsing. Together with the import rule, no workspace file can be shown by a library module's path, and no import can reach a workspace `prism/` directory.
- **Library nodes.** `SourceNode` gains `origin: SourceOrigin::{Workspace, Library}`. A library node's key is the virtual relative path `prism/<name>.prism`, which never equals a workspace path (those are absolute since `from_entries` makes them so) and which is exactly how diagnostics and runtime errors show it. Library nodes are never read from disk, canonicalized, root-discovered or boundary-checked. Their imports resolve only within the library: a sibling `core` names `prism/core`, `/prism/x` names `prism/x`, and anything else is `E1002`.

The zero-marker invariant holds: nothing on disk marks the library, and the workspace root still comes from the entry file's path and package. Kotlin's `kotlin.*` packages, Java's `java.*` and Rust's `std` are reserved the same way.

Compile-time cost: one segment comparison per import and per package declaration.

Rejected:
- A separate syntax for library imports (`import prism:ui`, `use std::`): a second import form for one package.
- Letting a workspace `prism/` directory shadow the library: the same import would mean different code in different workspaces.
- Reserving every module path starting with `prism`, which would break a root file `prism.prism`.

### D3. Library files are ordinary units, keyed like workspace files; no precompiled ABI (autonomous)

Library files go through `extract_abis`, `elaborate_file`, instance planning and code generation like any node. Their check entries are keyed by the same inputs (module path, source text, imports' ABI hashes, compiler identity), so they are shared between every workspace on the machine: the library is checked once per compiler build and cache, then reused. Their ABI hashes are hashed like any file's. Being in the executable, they are also covered by the compiler identity. That is redundant, and it lets tests inject modules (D13) without any special key. Prelude ABI hashes join a workspace file's check key after its explicit imports (D4).

Only the reached modules are parsed. In Phase 1 of every process, that means the prelude modules plus the imported ones. Parsing and extracting about 300 lines of library is a few hundred microseconds, small next to the 1.8 ms startup the `codegen-core` benchmark measures, and the startup benchmark is re-run (task 9.3). With the cache, a library body is never re-checked or recompiled. Without it (`--no-cache`), a run re-checks the prelude, a few dozen small bodies, most of them generic and checked once each. `/prism/core` has no bodies at all, since its declarations are intrinsic.

Compile-time cost: linear in the size of the reached modules, once per process for Phase 1 and once per cache for Phase 2 and code. Nothing is per workspace file.

Rejected:
- An ABI serialized at build time and embedded: a second ABI codec and a source of truth that must match the parse. It saves well under a millisecond, and the `.abi` cache entries of the `abi-cache` follow-up will serve library and workspace files alike.
- Trusting the library and skipping its Phase 2: `prism run` needs its MIR and records anyway, and the cache already makes it free.
- Hashing a library version instead of the sources: a version a developer can forget to bump.

### D4. A prelude of `/prism/core` and `/prism/collections`, one shared scope, found last (autonomous)

Every workspace file implicitly imports the prelude modules, in `PRELUDE` order, after its explicit imports. Library files do not, and import what they use, so the library's own dependency order is explicit.

- **Graph.** `SourceNode` gains `prelude_imports: Vec<PathBuf>`, set for workspace nodes, separate from `direct_imports` so the type checker can tell them apart. Topological order and instance planning treat them as direct imports.
- **One scope per process.** The prelude modules' ABIs are turned once into a `PreludeScope` (names, extensions and members, the conversion `ingest_file_abi` does), held in an `Arc` and given to every `TypeChecker` (`TypeChecker::with_prelude`). A name lookup that misses the scopes and the import layer consults it, and so does an extension lookup that misses the file's own and imported extensions. A file therefore costs one pointer copy, and nothing proportional to the prelude.
- **Shadowing.** A prelude name is shadowed by any local, parameter, top-level declaration or explicit import. A shadowing declaration is not `E4010`, and a name exported by both an import and the prelude is not `E4012`. This is today's rule for `print` and `listOf`, made general. An explicit import of a prelude module behaves as any import.
- **Keys.** The prelude modules' ABI hashes follow the explicit imports' in the check key, so changing the prelude re-checks its users (only possible with injected test modules, D13, since the compiler identity changes otherwise).

Kotlin's default imports (`kotlin.*`, `kotlin.collections.*`) are found after explicit imports, the same rule. `/prism/collections` is in the prelude because `listOf` and the list members are used everywhere, and `/prism/compose` joins it in `compose-transform` (D14).

Compile-time cost: building the prelude scope is linear in the prelude's ABI, once per process. Per file, a name or extension lookup that misses everything else costs one more hash lookup.

Rejected:
- Ingesting the prelude ABI into each checker: time proportional to files × prelude size, which `compile_timing.rs` would show on large workspaces.
- One `prism/prelude` module re-exporting others: Prism has no re-exports.
- An explicit `import /prism/compose` in every composable file (D2 of `compose-architecture`).
- Making prelude names unshadowable: adding a library function would break programs declaring that name.

### D5. `external fun`: a C symbol with Prism's own calling convention, checked against the runtime (autonomous)

An external is a body-less declaration whose name is its C symbol: `private external fun prism_rt_exit(status: Int): Nothing`. The typed tree records its calls as `Callee::External(DeclarationPath)` (new in `prism-typed-tree` and `prism-mir`), and the backend declares the symbol as an import and calls it directly with the target's C calling convention. Externals have no slot, no code and no reload. The runtime is never reloaded (`codegen-core` D17), so this is the call path runtime functions already take.

- **Machine signature.** It is the one `lower.rs::signature` gives a Prism function of the same signature, so there is a single convention to learn and implement. Scalars travel by value (`Int` as `i64`, `Double` as `f64`, `Bool` as `i8`, `Char` as `i32`). Values held in memory and references travel as pointers, a result held in memory as a pointer passed first, and `Nothing` means the function never returns. A by-value parameter gives the runtime ownership, as it gives a Prism callee. A function value is the 16-byte descriptor-and-environment value of `closures`, passed by address. The runtime calls it through its descriptor's slot and releases it through its drop entry, which is what `compose-runtime` needs for handlers. The existing `prism_rt_int_to_string(destination, value)` already has exactly this shape.
- **Restrictions** (`E4029`). Not `public`: a public wrapper keeps runtime symbols out of the library's API and ABI. Not generic, since one symbol has one machine signature. Not an extension, not `compose`, no defaults: a wrapper does each. No reference in the result, since no elision rule can make the runtime's answer safe. `package` visibility is allowed so that files of one library package share an external.
- **Checked ABI** (`E6005`). `UnitContext::external_function(symbol, signature)` looks the symbol up in `RUNTIME_FUNCTIONS` and compares the derived parameter and result types and the never-returns flag with the entry's `RuntimeType` list. A missing symbol or a difference is `E6005` at the declaration. The compose and UI crates add their own tables to the same lookup later. The library test (D13) compiles every external of the library, so a mismatch fails `cargo test` before any user can see it.

Kotlin has the same keyword for the same idea (Kotlin/Native's `external fun` with `@SymbolName`). Naming the external by its symbol needs no annotation syntax, and a symbol that is not a valid Prism identifier can get an explicit form later.

Compile-time cost: one table lookup and a signature comparison per external a unit calls.

Rejected:
- Symbols derived from module and name (`prism_system_exit`): renaming or moving a declaration silently breaks the link.
- An annotation or string for the symbol (`external("...")`): no annotation syntax exists yet.
- A C-only convention with aggregates split into scalars: two conventions to keep in step for no gain.
- Public externals: runtime names leak into the API.
- Checking externals at `prism check`: it needs layouts, which only the backend computes.

### D6. `intrinsic`: declared signatures for compiler-lowered calls, verified against a catalog (autonomous)

`intrinsic fun` and `intrinsic val` declare, without a body, what the compiler lowers. The type checker keeps an **intrinsic catalog** (`intrinsics.rs`, new). It maps a declaration path (module, receiver type constructor, name) to a `BuiltinFunction` and the canonical text of the expected signature (`fun Int.toString(): String`). Checking a library file compares each intrinsic's rendered signature with the catalog text, and an unknown path or a different text is `E4029`. So the declaration, the code that types its calls and the code that lowers them cannot drift apart. Calls emit `Callee::Builtin(..)` as today, so MIR, the borrow checker and the backend do not change for them.

Most intrinsics are **signature-typed**: their calls are checked exactly as calls of the declaration, by the ordinary function and extension paths (receiver modes, generic inference, `E4004`), which `collections` D6 already did with its table. Three are **rule-typed**, because their behavior needs a feature Prism lacks:

| Intrinsic | Declared | Rule | Retired by |
|---|---|---|---|
| `print`, `println` | `fun <T> print(value: &T)` | a text-form argument (D9); `println` also takes none | traits (`T : TextForm`) and overloading |
| `listOf` | `fun <T> listOf(element: T): List<T>` | any number of elements | `vararg` |
| `contains`, `indexOf` | their signatures | also `==` on `T` at the call | traits (`T : Equatable`) |

The declared signature of a rule-typed intrinsic is the shape its future declaration will have. It is still the catalog's key and the ABI's entry, and the rule is implemented by the existing checkers (`infer_output_call`, `collections`' `listOf`), selected by `BuiltinFunction`.

`intrinsic val` is the one new declaration form, an extension property (`public intrinsic val <T> List<T>.size: Int`), because `length`, `size` and `indices` are read without parentheses. It is accepted only with `intrinsic` (`E2001` otherwise), so ordinary extension properties stay a separate decision (a follow-up).

Compile-time cost: one catalog lookup and a string comparison per intrinsic declaration, in the library's own check (cached). Calls cost what they cost today.

Rejected:
- Intrinsics typed only by hand-written rules, with the declaration documenting them: the declaration and the checker would drift.
- No declaration at all for rule-typed built-ins (keeping `print` and `listOf` in the type checker): two mechanisms for "found last".
- A `vararg` or overloading feature now, to make `listOf` and `println` signature-typed: language features of their own, recorded as follow-ups.
- Intrinsic types (`intrinsic struct State<T> : Copy`) now: `state-handles` needs them first and owns their representation, on this catalog.

### D7. Intrinsic extensions are members; other library extensions are ordinary (autonomous)

An intrinsic extension is a member of its receiver type. It is found before every extension, after fields and the built-in `clone()` and `unbox()`, and a workspace extension of the same name on that receiver type constructor is `E4010` (`UncallableExtension`). This is the rule `extension-methods`, `runtime-strings-and-drops` and `collections` already apply to `toString`, `length` and the list members, now read from declarations instead of a hand-written list. The prelude scope (D4) holds the members in a map from (receiver key, name) to `Extension`, consulted in `infer_method_call` and `build_member_access` before `find_extension`, and by the `E4010` check of each extension declaration.

Library extensions that are not intrinsic (`forEach`, `map`, ...) are ordinary prelude extensions, which a file's own or imported extension shadows. Adding a helper to the library then never breaks a program that declared its own, which is why Kotlin lets explicit imports beat default imports. Members belong to the type's representation, and the compiler lowers all of them, so they may not be redefined, as in Kotlin, where a member beats an extension.

Compile-time cost: one hash lookup per method call and per extension declaration, as today's special cases.

Rejected:
- Every library extension on a built-in type as a member: the next helper added would break user code.
- Every library extension as shadowable: `fun Int.toString()` in one file would change what `n.toString()` means there.
- A separate `member` modifier: `intrinsic` already says the compiler owns the operation.

### D8. What moves into the library now, and what stays (autonomous)

Moves now, as declarations:
- `print`, `println`;
- `toString()` of the four scalars;
- `String.length`;
- `listOf` and the list members.

Their typing and lowering stay in the compiler, but their names, signatures, visibility and lookup come from the library. The special cases `output_builtin`, the scalar `toString()` branch of `infer_method_call`, the `length` branch of `build_member_access` and `collections`' member table are replaced by the catalog and the prelude scope.

New Prism code: the higher-order helpers (D11) and `exitProcess` (D10).

Stays in the compiler, for now:
- the built-in types (`Int`, ..., `String`, `List`, `Heap`, `Unit`, `Nothing`, and `Range`, the one nominal);
- `clone()`, which is defined for every `Clone` type by its marker and cannot be declared per type before traits;
- `Heap(...)` and `unbox()`, since the type and its constructor share a name and the parser rejects `Heap` receivers (`heap-boxes`);
- indexing, operators, `==` and templates.

`Range` as a library struct (a "language item" the compiler knows by path), `Heap` declared in `/prism/core`, and `abs`/`min`/`max` are follow-ups. Math needs a decision on how one name serves `Int` and `Double` without overloading, which a `minInt`-style name now would only have to reverse.

Compile-time cost: none beyond D4 and D6.

Rejected:
- Moving only the new helpers and keeping today's built-ins as tables: two mechanisms forever.
- Moving everything, `Range` and `Heap` included: changes to the parser and to struct paths that buy nothing yet.

### D9. `print` of any type with a text form, by the template rule (autonomous)

`print(x)` and `println(x)` accept exactly what a template part accepts, and print what `"$x"` would. `check_printed_argument` delegates to the template-part check (`check_template_part`, factored into `check_text_form`).

- A `String` is borrowed and a scalar copied out, as today, so the typed tree of every existing call is unchanged.
- Any other type becomes `print(&x.toString())`: the `toString()` call is resolved as `x.toString()` would be (members, then the file's, imported and prelude extensions), called on a shared borrow, and its result is a temporary of the statement, dropped after printing. MIR and the backend see a print of a `&String`, which they already lower.
- Nullables, `Unit`, function values and type parameters are `E4026`, with the template's help, instead of `E4004`.

Kotlin prints `null` for a null argument. Templates chose to reject nullables (`string-templates` D6 to D8), and `print` follows them, so the two never disagree. Printing nullables is a follow-up for both.

Compile-time cost: one method resolution per non-scalar print argument, as for a template part.

Rejected:
- Keeping `print` to scalars and strings: an arbitrary difference from templates, and the follow-up that asks for this.
- A runtime formatting protocol: needs traits.

### D10. `/prism/system` with `exitProcess`, the first external (autonomous)

`/prism/system` declares `public fun exitProcess(status: Int): Nothing = prism_rt_exit(status)` over `private external fun prism_rt_exit(status: Int): Nothing`. The new `crates/prism-runtime/src/process.rs::prism_rt_exit(status: i64) -> !` flushes the output and calls `std::process::exit((status & 255) as i32)`. Like a runtime error, it runs no drop and no allocation check. Under `prism run --watch` it calls the stop hook that `hot-reload` installs for panics, so the session reports the run's end and waits, as after `main` returns. The module is named after Kotlin's `kotlin.system.exitProcess` and is not in the prelude, as in Kotlin.

It gives the external form a real user with an executable fixture, and it closes the `codegen-core` D13 follow-up. Taking the low 8 bits is what POSIX `exit` does, and doing it in the runtime keeps statuses the same on every platform.

Compile-time cost: none.

Rejected:
- `main(): Int`, which `E6003` forbids today and which would be a separate change of the entry point.
- A runtime error for a status outside `0..255`: an external has no caller location, and the low bits are the familiar behavior.
- Leaving externals without a real user until `compose-runtime`: the form would ship untested end to end.

### D11. The helper set (autonomous)

`/prism/collections` gains `forEach`, `forEachIndexed`, `map`, `filter`, `any`, `all`, `none`, `count`, `indexOfFirst`, `fold`, `sumOf` and `joinToString`, with Kotlin's names. They are written over `for` loops and the members, and their function parameters borrow each element (`(&T) -> ...`). `filter` is `consuming`, as `collections` D17 sketched: it reverses its list and pops elements into a new one, so kept elements move without a clone, against nothing in the explicit-clone rule of `ownership`. `joinToString(separator: &String = ", ", transform: (&T) -> String)` needs its transform, since a type parameter has no text form before traits. `sumOf` adds `Int`s and overflows as `+` does. None of these function parameters is `escaping`, so each closure borrows its captures.

A closure passed to `forEach` is not a composable context, so composables are emitted from `for` loops (`collections` D17). Kotlin needs `inline` for that.

Compile-time cost: each helper's body is checked once per cache. Each use is one generic call, instantiated once per type-argument list per program (`monomorphization`).

Rejected:
- A borrowing `filter` that clones the kept elements: a hidden clone.
- `joinToString` without a transform, through the text form: it needs a `TextForm` bound.
- `first`, `last`, `find` returning `(&T)?`, sorting and `sum()`: `collections` follow-ups that need their own decisions.

### D12. Syntax and codes of the library-only forms (autonomous)

- **Modifiers.** `external` and `intrinsic` are contextual modifiers, like `consuming`: recognized only at the start of a top-level declaration, after any visibility and directly before `compose`, `mut`, `consuming`, `fun` or `val`. Elsewhere they are names. The order is visibility, then `external` or `intrinsic`, then `compose`, then the receiver mode. `intrinsic compose fun remember(...)` is the form `compose-transform` will use.
- **Bodies.** A `fun` without a body becomes `E2001` unless it is `external` or `intrinsic`, and those must have none. Today the parser silently accepts a body-less `fun`, which nothing uses, so the only spec text it changes is the span scenario of "Lexical Tokenization and Escape Sequences".
- **Codes.** `E1008` `ReservedPackage` (resolver). `E3002` `LibraryOnlyDeclaration`: a workspace file using either modifier, raised in Phase 1 by `FileAbi::extract_with_definitions`, which gains an `in_library` flag, so no ABI ever carries a workspace intrinsic. `E4029` `InvalidLibraryDeclaration`: an external breaking D5's rules, or an intrinsic missing from the catalog or with another signature, raised by the type checker of the library file. `E6005` `ExternalMismatch` (backend). Each is the next free code of its stage. `E4026` and `E4010` are widened.

Compile-time cost: one token comparison per declaration start.

Rejected:
- Hard keywords: they would break programs using `external` or `intrinsic` as names.
- Rejecting library-only forms in the parser: the parser does not know the file's origin, and Phase 1 does.
- One code for all library-declaration errors: a workspace author sees `E3002` only, and library authors get precise `E4029` and `E6005`.

### D13. How the library is checked and tested (autonomous)

- **`library_tests.rs`** (new, `crates/prism-cli/tests/`) builds a graph of every embedded module (`DependencyGraph::from_library`) and runs Phase 1 and Phase 2. It formats each source with `prism_formatter` and compares. It compiles every module with the backend, so each external meets `E6005`'s check. It asserts that the prelude modules' exported names are disjoint and that the static module list equals the `.prism` files of `crates/prism-library/prism/`.
- **Injected modules.** `CheckOptions` gains a `library: Arc<Library>`, which defaults to the bundled library. The fixture runner reads a `// library: <dir>` header that adds the case's `<dir>/prism/*.prism` as extra library modules with `Library::with_modules`. The library-only diagnostics (`E4029`, library imports, positions in library files) therefore have real-source fixtures, and only `E6005` is tested by `prism-codegen` tests over injected modules, listed in `UNFIXTURED_CODES` with that reason. Injection is not reachable from the `prism` command line (D1).
- **Fixtures** under `crates/prism-cli/tests/fixtures/standard-library/` for each scenario, plus the cases of the other deltas under their specs. Executable ones run under `--check-allocations`.
- **Timing.** `compile_timing.rs` gains a shape of N workspace files that each call prelude functions and helpers, so the prelude's per-file cost stays flat (D4), and the startup benchmark `run_timing.rs` is re-run against its 15 ms target.

Compile-time cost: test time only.

Rejected:
- `prism check` on the library's directory: its files declare `package prism`, which `E1008` rejects in a workspace, and the embedded copy is what ships.
- An environment variable for injection: it would reach users.

### D14. `/prism/compose` and `/prism/ui` are created by the changes that fill them (autonomous)

This change creates no stub modules. `compose-transform` creates `/prism/compose` with `remember` and `key` as intrinsic composables and adds it to `PRELUDE`. `state-handles` adds `State<T>`, `MutableState<T>` and `mutableStateOf` there, with intrinsic types on D6's catalog. `ui-toolkit` creates `/prism/ui`, its `Node` intrinsic and its externals of `prism-ui`. The scenario "Compose primitives need no import" of `docs/architecture.md` §5 then moves with `/prism/compose`, and the rest of that requirement is implemented here (task 9.5). An empty prelude module would specify nothing and would have to be re-specified when it is filled. Adding a module is one list entry and one file.

Compile-time cost: none.

Rejected: empty `/prism/compose` and `/prism/ui` stubs now.

### D15. Reports and paths

`prism check`'s report counts and lists workspace files only, so `Total units` keeps meaning the user's program, and `--explain-cache` lists library files first, in check order, since their cache use is part of the explanation. Library files are shown by module path plus `.prism` everywhere: diagnostics, `--explain-cache`, runtime error locations and code units' file names. Error rendering takes the source from the node (`SourceNode::source`) instead of re-reading the file, which library nodes need and which also removes a race with edits.

## Risks / Trade-offs

- **Rule-typed intrinsics have declarations that do not fully describe them.** `println()` with no argument and `listOf(1, 2, 3)` do not match their declared single parameter. Mitigation: the catalog marks them, the spec lists them, and each disappears when overloading, `vararg` or traits land.
- **The prelude grows with compose.** Each added module adds to the prelude scope's construction, once per process, and to every check key. It is still linear and per process, measured by the timing shape of D13.
- **A library bug looks like a compiler bug to users.** A runtime error located in `prism/collections.prism` points into code users cannot open. Materializing the sources for editors (a follow-up) and the library tests make this rare and readable.
- **`E6005` needs a full compile.** A wrong external is found by the backend, not by `prism check`. Only the library declares externals, and its test compiles every one.
- **Snapshot churn.** `prism check --explain-cache` outputs and `prism abi` dumps gain library lines. Typed-tree and MIR snapshots of `print` calls do not change (D9). Calls of `toString()` and the list members keep `Callee::Builtin`.
- **Typechecker unit tests need the prelude.** Tests that build a `TypeChecker` directly and use `println` must install the prelude scope. A test helper builds it from the bundled library (task 4.7).
- **Hot reload must ignore library nodes.** The watch session (`hot-reload`) must never invalidate or watch them. They come from `Library`, not from its `SourceStore`.

## Migration Plan

No user program changes meaning, except:
- a body-less `fun` is now `E2001`;
- `print` of a struct with `toString()` now prints instead of reporting `E4004`, and `print` of a nullable or function reports `E4026` instead of `E4004`;
- `val p = println` reports `E4013` instead of `E4001`;
- a workspace `package prism...` is `E1008`.

Fixtures pinning those codes are updated with the scenarios. `--explain-cache` and `prism abi` snapshots are re-blessed and read against the new scenarios.

## Open Questions

None blocking. The follow-ups in the proposal's "Not in scope" go to `ROADMAP.md` at archive.
