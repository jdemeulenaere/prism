# Proposal: core-library-in-prism

## Why

The post–Milestone 2 review agreed that the standard and compose libraries become "as much as possible ... normal prism code that is imported like any other code", over the smallest possible intrinsic core, with no `unsafe` layer, so `List`, `String` and `Heap` stay Rust-backed (item **A4** of `docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20). Item 10 of "After Milestone 2: the review round" in `ROADMAP.md`, `library-in-prism`, does that once `operator-conventions` (L1), `drop-and-clone-hooks` (L2) and `vararg` (L4) have landed.

Inventorying every `intrinsic` and `external` declaration and every compiler special case for library names (design D1, D10) shows two very different halves:

- **The core and collections modules and the unit properties** need one small language addition (extension properties with getters) and one rule change (which extensions are members of the built-in types).
- **The compose runtime and toolkit handles** need several decisions of their own: field visibility so that a library struct can wrap a runtime handle without being forged, the composer primitive, markers for composables that run in their caller's group or never skip, and declared replacements for the name tests on `Composition` and the state types.

So `library-in-prism` is split into two sequential changes. This one, **`core-library-in-prism`**, comes first. **`compose-library-in-prism`** follows it, starting from the classification recorded in this change's design (D10).

## What Changes

- **Extension properties with getters**, Kotlin's form: `public val Int.dp: Dp get() = Dp(this)`, or `get() { ... }`, with the getter on the same line or the next one. They are read-only (no setters), take their receiver as a shared borrow and always declare their type. Their type parameters must occur in the receiver. They resolve exactly like extension functions: fields and members first, then the file's own extensions, then its imports'. A read is a call of the getter. They work in every file. Until now `intrinsic val` was the only extension property, so this closes the `standard-library` follow-up "Extension properties outside the library".
- **`/prism/core` declares every member of the built-in types**, intrinsic or written in Prism, and is the only member module. The `List<T>` members move there from `/prism/collections`. `/prism/collections` becomes an ordinary module that holds `listOf` and the helpers and is imported as today. So a program that imports nothing no longer parses `/prism/collections`. Every extension that `/prism/core` declares on a built-in type is a member: it is in scope in every file with no import, and it comes before extensions (`E4010` for a workspace extension of its name).
- **Members written in Prism.**
  - `toString()` of `Int`, `Double`, `Bool` and `Char` becomes a Prism function over the runtime's existing `prism_rt_*_to_string` entries, declared as private externals.
  - `List<T>.isEmpty()` and `isNotEmpty()` are written over `size`.
  - `List<T>.indices` becomes a Prism extension property, `0..<size`.
  - The type checker's built-in `toString()` path and the built-ins `ToString`, `ListIsEmpty`, `ListIsNotEmpty` and `ListIndices` are removed. These calls become ordinary calls of their declarations in the typed tree, MIR and dumps.
- **`Int.dp` and `Int.sp`** become Prism extension properties of `/prism/compose/ui` (`get() = Dp(this)`). They are ordinary imported extensions, as in Compose: a file's own `Int.dp` now shadows the import instead of being `E4010`. A unit read costs a call instead of an inline construction. The built-ins `DpOfInt` and `TextUnitOfInt` are removed.
- **What stays intrinsic, and why** (D2):
  - `String.length` and `List.size`, which read the Rust-backed representation.
  - `add`, `insert`, `removeAt`, `removeLast`, `reverse` and `clear` of `List<T>`, which move elements of a Rust-backed buffer for any element type. Externals cannot be generic, and there is no `unsafe`.
  - `contains` and `indexOf`, which need `==` on `T` until traits exist.
  - `print` and `println`, which take an argument of any type with a text form, as Kotlin's `Any?` does. That needs `Any`, traits or overloading, so this settles `vararg` D11 for them.
  - The built-in types, `Range`, `Heap(...)`, `unbox()`, `clone()`, the operators of built-in types (string indexing and concatenation included), structural `==` and templates stay compiler built-ins with no declaration.
- **The resulting catalog** (D9) has 12 entries for the core: `print`, `println`, `String.length`, and nine `List<T>` members. The compose entries are unchanged here.
- **ABI**: every public or `package` extension property is exported, with its kind (plain or intrinsic), and covered by the ABI hash. A getter's body is not in the ABI, so editing it recompiles nothing downstream. `prism abi` prints `public val Int.dp: Dp`.
- **Formatting** prints a getter on its property's line when it fits, and otherwise on the next line, indented one level.
- **No new error code**: `E2001`, `E4003`, `E4004`, `E4006`, `E4010` and `E4012` cover every mistake.
- **BREAKING (small)**:
  - A workspace `fun Int.dp(): Int` is accepted, shadowing the import, where it was `E4010`.
  - A workspace extension `isEmpty`, `isNotEmpty` or `indices` on `List` is still `E4010`, now naming `/prism/core`.
  - Dumps show `call prism/core::Int.toString` instead of `call builtin toString`.
- **Not in scope**, left for follow-ups:
  - Everything in `/prism/compose/runtime` and the toolkit's intrinsics (`compose-library-in-prism`).
  - `var` extension properties with setters.
  - `compose` or context-parameter extension properties.
  - Moving `String.length`, `List.size` or `removeLast` to Prism, and `Range` or `Heap` as library declarations.

## Capabilities

### New Capabilities

None. Extension properties belong to `extension-functions` (methods are extensions), and the rest changes existing capabilities.

### Modified Capabilities

- `extension-functions`: extension properties with getters: resolution, the getter's receiver, reads as calls, the rules on declarations, and their compile-time cost. Members of the built-in types written in Prism take precedence like intrinsic ones.
- `syntax`: extension property getters, the contextual `get`, and type parameters that occur in the receiver. The extension-property requirement covers both intrinsic and getter forms.
- `standard-library`:
  - `/prism/core` as the only member module, declaring every member of the built-in types, and members written in Prism.
  - `/prism/collections` as an ordinary module.
  - The unit properties as ordinary Prism extensions.
  - The list operator intrinsic in `/prism/core`.
  - Only the core module parsed for a program without imports.
- `type-system`: scalars' `toString()` is a member declared by `/prism/core`.
- `collections`: the members of `List<T>` are declared by `/prism/core`. `isEmpty`, `isNotEmpty` and `indices` are written in Prism.
- `typed-tree`: scalars' `toString()`, `isEmpty`, `isNotEmpty`, `indices` and property reads are calls of their declarations.
- `mir`: those calls are no longer built-in callees; unit properties are getter calls, not constructions; extension property reads are calls.
- `compilation-and-abi`: one member module, instances of generic members, and extension properties in the ABI.
- `build-cache`: check keys append only `/prism/core`'s ABI hash; cache explanations list it alone for programs without imports.
- `formatting`: extension property getters are printed.

## Impact

- `crates/prism-ast` (`ExtensionPropertyDecl::getter`), `crates/prism-parser/src/parser.rs` (`parse_extension_property` for non-library properties, the contextual `get`), `crates/prism-formatter` (getter layout, `verify.rs`).
- `crates/prism-abi/src/signature.rs` (`ExportedSymbol::ExtensionProperty` gains a kind; non-intrinsic properties exported and hashed; `prism abi` text).
- `crates/prism-typechecker`:
  - `checker.rs`: registration, ingestion and resolution of extension properties; `infer_to_string_call` and the scalar `toString` path removed; member precedence for Prism members.
  - `members.rs`: the member scope takes every extension of the member module on a built-in type.
  - `intrinsics.rs`: catalog entries removed, and the list members' module changed to `prism/core`.
- `crates/prism-typed-tree` and `crates/prism-mir` (`BuiltinFunction` variants removed and codec tags retired), `crates/prism-mir/src/build/call.rs` (no unit construction), `crates/prism-codegen/src/lower.rs` (the `ToString` lowering removed).
- `crates/prism-library` (`MEMBER_MODULES` is `prism/core` alone), `crates/prism-resolver` (member imports), `library/prism/core.prism`, `library/prism/collections.prism` (imports `core`) and `library/prism/compose/ui.prism` (`dp`, `sp`).
- Fixtures under `crates/prism-cli/tests/fixtures/{extension-functions,syntax,standard-library,type-system,collections,typed-tree,mir,compilation-and-abi,build-cache,formatting}/`. Re-blessed snapshots that showed the removed built-ins. `library_tests.rs`, `cache_tests.rs`, and `compile_timing.rs` (shape `extension-properties`).
- `openspec/specs/diagnostics/spec.md` (`E2001`, `E4006` and `E4010` rows), `ROADMAP.md` (item 10 split into `core-library-in-prism` and `compose-library-in-prism`).
