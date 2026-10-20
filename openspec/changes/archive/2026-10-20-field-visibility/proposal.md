# Proposal: field-visibility

## Why

The post–Milestone 2 review agreed that the compose library becomes ordinary Prism code over a minimal intrinsic core (item **A4** of `docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20: "as much as possible ... normal prism code that is imported like any other code"). Its second half, `compose-library-in-prism`, rewrites `Composition`, `RecomposeScope`, `NodeId` and `HandlerId` as library structs that wrap a runtime handle (`core-library-in-prism` D10). Today any program could construct such a struct from any `Int` or read its handle, because every field of a struct is visible wherever the struct is. That is why `compose-transform` D16 made `Composition` an intrinsic struct ("Rejected: forgeable library structs").

Prism has no way to hide a field. `ROADMAP.md` lists this gap twice:

- "Field visibility (`private var balance`), the encapsulation boundary for extension functions" (from `extension-methods`);
- the `ui-toolkit` follow-up "Field visibility, so that `ModifierElement`'s payloads and the element blocks are not readable or constructible by programs".

Field visibility is a language feature of its own, useful to every program, and small. So it goes first, as item 10b of "After Milestone 2: the review round", and `compose-library-in-prism` (10c) builds on it.

## What Changes

- **Visibility modifiers on struct fields**, Kotlin's form: `struct Account { private var balance: Int, val owner: String }` and `struct Session(private val id: Int)`.
  - A field takes `private`, `package` or `public` before `val`/`var`.
  - Without a modifier, a field is visible wherever its struct is, as Kotlin's members are public by default, so every existing program keeps its meaning.
  - `private` means the struct's file, which is Prism's meaning of `private` for top-level declarations (Kotlin's file-private top level), so extension functions in that file are the type's methods.
  - `package` means the struct's package directory.
  - A field is never more visible than its struct.
- **What a hidden field forbids**, each `E4007` (`VisibilityViolation`, widened), at the field's name or at the construction:
  - reading, writing or borrowing it, also through a bare name in an extension body;
  - constructing the struct where any of its fields is not visible (Rust's rule): a struct with a `private` field can be built only in its own file, so it cannot be forged;
  - naming it in a pattern, a named sub-pattern or a destructuring entry. A positional struct pattern names every field, so it is rejected where any field is hidden. Named patterns and destructuring declarations that leave hidden fields out stay valid.
- **Copies, `clone()`, drops, structural `==` and text forms are unchanged.** They are the type's own operations, not field accesses.
- **Resolution counts only visible fields.** Where a field takes precedence over a member or an extension, or makes an extension or extension property of its name `E4010`, only a field visible in that file counts. So another file can declare `fun Session.id(): Int`, and `s.id()` there calls it. This closes the ROADMAP item from `extension-methods`.
- **ABI**: each exported field carries its visibility, covered by the ABI hash, and `prism abi` prints it (`private val id: Int`). Hidden fields stay in the ABI, since importers lay values out. Changing a field's visibility re-checks importers and changes no layout, contract or type fingerprint.
- **Formatting** keeps field modifiers as written.
- **The library uses it**: `Modifier`'s fields (`count`, `first`, `second`) and `ModifierElements`' fields (`e0` to `e7`) become `private`. Programs build chains only through the modifier functions, which closes the element-block half of the `ui-toolkit` follow-up. `ModifierElement`'s variants stay constructible, since enum payloads have no visibility.
- **No new error code**: `E4007` covers every hidden-field use, and `E2001` covers misplaced modifiers.
- **BREAKING (small)**: a program that constructs or reads `Modifier`'s or `ModifierElements`' fields is now rejected. No example or fixture does.

Not in scope, left for follow-ups:

- visibility of enum variants or payloads;
- a separate constructor visibility (Kotlin's `private constructor`) for structs whose fields are public;
- `protected` and `internal`;
- read-only views of `var` fields (Kotlin's `private set`).

## Capabilities

### New Capabilities

None. Field visibility belongs to the type system and syntax.

### Modified Capabilities

- `syntax`: visibility modifiers on struct fields, in block and shorthand structs.
- `type-system`: what a field's visibility allows (reads, writes, borrows, construction), the visible field set used by resolution, and `E4007` for hidden fields.
- `extension-functions`: only fields visible in the file take precedence over extensions or conflict with them.
- `patterns`: struct patterns and destructuring declarations respect field visibility.
- `compilation-and-abi`: field visibility in the ABI and its hash.
- `formatting`: field modifiers are printed.
- `ui`: modifier chains are built only through the modifier functions.

## Impact

- `crates/prism-ast/src/decl.rs` (`StructField` gains `visibility`), `crates/prism-parser/src/parser.rs` (field modifiers in both struct forms), `crates/prism-formatter` (printing and `verify.rs`).
- `crates/prism-abi/src/signature.rs` (`ExportedSymbol::Struct` fields carry visibility; hash; `prism abi` text) and the ABI codec of the build cache.
- `crates/prism-typechecker/src/checker.rs` (field lookup by visibility in member access, assignments, places, implicit `this`, construction and the extension checks of `register_extension_decl`) and `checker/patterns.rs` (struct patterns and destructuring).
- `library/prism/compose/ui.prism` (`Modifier` and `ModifierElements` fields).
- Fixtures under `crates/prism-cli/tests/fixtures/{syntax,type-system,extension-functions,patterns,compilation-and-abi,formatting,ui}/`, `cache_tests.rs`, `fmt_cli_tests.rs`, and `compile_timing.rs` (shape `field-visibility`).
- `openspec/specs/diagnostics/spec.md` (the `E4007` and `E2001` rows), `ROADMAP.md` (items 10b and 10c, and the two follow-ups closed).
