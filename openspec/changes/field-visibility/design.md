# Design: field-visibility

## Context

See `proposal.md` for the motivation. This change is item 10b of "After Milestone 2: the review round" in `ROADMAP.md`. It is the language prerequisite of `compose-library-in-prism` (10c), the compose half of agreed item **A4** of `docs/milestone-2-review-notes.md`: "as much as possible ... normal prism code that is imported like any other code" (user, 2026-10-20), with no `unsafe` layer. `core-library-in-prism` D1 and D10 named field visibility as what lets a library struct wrap a runtime handle without being forged.

The user is away. Every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Decisions the user took in the review are marked **(user direction 2026-10-20)**. Compile speed is a primary criterion (AGENTS.md §5), so each decision states its compile-time cost.

**Baseline.** Proposed on top of `b56208a` ("Warn about the missing timeout command on macOS in the playbook"). Three packets are open, and the order of `ROADMAP.md` lands them before this one:

- `drop-and-clone-hooks` (item 8, implemented, under review);
- `vararg` (item 9, proposed);
- `core-library-in-prism` (item 10a, proposed).

This packet modifies no requirement of the main specs: it only ADDS requirements, so it cannot conflict with them. In particular, the field precedence rules of `extension-functions`, which `core-library-in-prism` modifies ("Members Take Precedence Over Extensions", "Extension Declarations Are Checked") and extends (its ADDED "Extension Properties Are Read Like Fields" and "Extension Property Declarations Are Checked"), are qualified by one ADDED requirement here ("Only Visible Fields Count"), rather than rewritten. The first task re-checks this.

**Baseline update (apply, on `d2cfc9cb`).** `drop-and-clone-hooks`, `vararg` and `core-library-in-prism` are archived, and so is nothing else since `b56208a`; three packets were proposed in between (`compose-library-in-prism`, `layout-primitive`, `layout-in-prism`), each building on this one, none touching the blocks it adds. This packet only ADDS requirements, and none of them exists in the main specs (no `syntax`, `type-system`, `extension-functions`, `patterns`, `compilation-and-abi`, `formatting` or `ui` requirement mentions field visibility; `diagnostics` still has `E4007` for `package` declarations only and `E2001` without field modifiers). "Only Visible Fields Count" matches the field rules as `core-library-in-prism` left them: "Members Take Precedence Over Extensions" counts a field first (and a member property as a field), "Extension Properties Are Read Like Fields" reads a property only when the receiver's struct has no field of the name, and "Extension Property Declarations Are Checked" makes a property named like a field `E4010`; each of these is about the field, so qualifying "field" with "visible in the file" is consistent for calls, property reads, bare names and both `E4010` rules, and a hidden field then falls through to the member property, extension property or extension of its name in the order those requirements give. Every name the tasks use exists: `StructField` (`crates/prism-ast/src/decl.rs`), `parse_struct` in `crates/prism-parser/src/parser.rs` (both struct forms), `ExportedSymbol::Struct` (`crates/prism-abi/src/signature.rs`, whose `fields` are `(name, is_var, type)` shared with `AbiTypeMembers::Fields`), `register_extension_decl` (also registering extension properties), `infer_member_access`, `check_take_apart_source` and the struct pattern and destructuring checks of `checker/patterns.rs` (`struct_sub_patterns`, `check_destructure`). The build cache stores no ABI symbol (it re-extracts them), so it has no ABI codec to update. The checker's field records live in the `TypeTable` of `prism-typed-tree`, which MIR, code generation and the instance records also read, so the visibilities go in a table of the checker's own (task 4.1), and the type table, its codec and type fingerprints stay as they are. `openspec validate field-visibility --strict` is clean.

Current state (main specs and code):

- A struct field is `val` or `var` with a name and a type (`StructField` in `crates/prism-ast/src/decl.rs`), visible wherever its struct is.
- Top-level declarations are `private` to their file by default, `package` to their package directory, or `public` (`syntax`, "Default Private Visibility and Export Modifiers"). Using a `package` declaration from another package is `E4007` (`VisibilityViolation`).
- An exported struct lists every field with its name, mutability and type (`ExportedSymbol::Struct::fields`), which importers use to type field accesses and to lay values out.
- A field takes precedence over members and extensions in calls (`extension-functions`, "Members Take Precedence Over Extensions"). An extension named like a field of its receiver struct is `E4010`.

## Goals / Non-Goals

**Goals:**

- A struct can hide fields from other files or packages, so that a library struct wrapping a runtime handle can be neither forged nor read outside its file.
- Kotlin's surface for fields, and Prism's existing meanings of `private`, `package` and `public`.
- Existing programs keep their meaning: an unmarked field behaves as today.
- Constant compile-time cost per field use, no new pass.

**Non-Goals:**

- Visibility of enum variants and payloads.
- A constructor visibility separate from field visibility (Kotlin's `private constructor`).
- `protected`, `internal`, and Kotlin's `private set`.
- Rewriting the compose handles: `compose-library-in-prism`.

## Decisions

### D1. Fields take `private`, `package` or `public`; unmarked fields are visible wherever their struct is (autonomous)

The modifier comes before `val` or `var`, in both struct forms:

```kotlin
struct Account { private var balance: Int, val owner: String }
struct Session(private val id: Int)
public struct Modifier : Copy { private val count: Int, ... }
```

**The default.** An unmarked field is as visible as its struct. Kotlin's members are public by default, and every program written so far relies on it. A field marked `public` says the same thing explicitly, as Kotlin allows.

**The modifiers.**

- `private` means the struct's file, which is Prism's `private` for top-level declarations and Kotlin's file-private top level. Methods are extension functions, so "the type's methods" are the extensions of its file. This is the "encapsulation boundary for extension functions" the ROADMAP item asks for.
- `package` means the struct's package directory, as for top-level declarations.
- A field's effective visibility is the narrower of its own and its struct's: a `public` field of a `package` struct is visible in the package only. No diagnostic is given for a modifier wider than its struct, as Kotlin gives none.

**Errors.** A modifier written twice, two modifiers, a modifier on an enum payload, and a modifier not followed by `val` or `var` (a shorthand field may omit `val`, `struct Id(id: Int)`, but not after a modifier, as Kotlin requires `private val` in a constructor) are `E2001`.

**Compile-time cost.** The parser tests one optional token per field.

Rejected:

- **Private by default**, as top-level declarations are: every existing struct used across files would break, and Kotlin's default for members is public.
- **Kotlin's class-private `private`**: Prism has no class bodies. Methods live in files, so file-private is the only boundary they can share.
- **Rejecting `public` on fields**: Kotlin code writes it.

### D2. Constructing a struct needs every field visible (autonomous)

A construction, positional or named, assigns every field, so it is rejected with `E4007` where any field is hidden, naming the first hidden field and its visibility. A struct with a `private` field can therefore be built only in its own file. This is what makes a handle unforgeable, and it is Rust's rule for struct literals.

**Compile-time cost.** One visibility comparison per field of a construction, done while its arguments are matched.

Rejected:

- **Kotlin's `private constructor`**: a second visibility to declare. Every handle type of the library hides its field anyway, and a struct with public fields and a private constructor is a follow-up if a program needs it.
- **Constructing with only the visible fields, hidden ones taking defaults**: fields have no defaults.

### D3. Reading, writing and borrowing a hidden field is `E4007` (autonomous)

The following are `E4007` where the field is not visible:

- `s.id`, `s?.id` and `s.balance = 1`, also through references and boxes;
- compound assignments, borrows `&s.id` and `&mut s.balance`, and paths captured by closures;
- a bare name that would read a receiver's field in an extension body or a lambda with a receiver.

The message names the field, its visibility and the file or package that declares it. Copies, moves of whole values, `clone()`, drops, structural `==`, hashing of kept values and text forms are not field accesses: they are the type's own operations, generated or declared in its file, and work everywhere.

A hidden field is not "no field". When nothing else resolves the name (no member, extension or outer binding, D5), the error is `E4007` for the hidden field, not `E4006`.

**Compile-time cost.** One visibility comparison per field access: the field record holds its visibility, and the checker knows the current file and package.

### D4. Patterns and destructuring name only visible fields (autonomous)

- A **positional struct pattern** (`Session(val id)`) gives every field, so it is `E4007` where any field is hidden.
- A **named sub-pattern** (`Account(owner = val o)`) or a **destructuring entry** (`val (owner) = account`) naming a hidden field is `E4007`.
- Named patterns and destructuring declarations that leave hidden fields out stay valid. The fields they leave out are matched by wildcards, as `patterns` already says, so exhaustiveness is unchanged.
- **Taking a value apart** this way (moving a visible non-`Copy` part out of an owned value) stays valid, with the hidden parts handled as any part a pattern leaves out, as Rust allows `let S { a, .. } = s` with private fields. A type that must not be taken apart declares a drop hook (`drop-and-clone-hooks` D5).

**Compile-time cost.** One comparison per field a pattern names, and one flag test per positional struct pattern.

### D5. Only fields visible in the file count for resolution (autonomous; closes the `extension-methods` follow-up)

Everywhere a field takes part in name resolution, only a field visible in the file of the use or declaration counts:

- the call precedence of "Members Take Precedence Over Extensions";
- the property reads of `core-library-in-prism` ("Extension Properties Are Read Like Fields");
- bare names in extension bodies;
- the `E4010` rule for an extension or extension property named like a field.

So `fun Session.id(): Int` declared in another file than `Session` is valid, and `s.id()` there calls it. In `Session`'s own file, the field still wins and such an extension is still `E4010`.

**Compile-time cost.** The existing field lookup gains one comparison.

Rejected: **hidden fields still shadowing extensions everywhere**. An extension could then be declared but never called outside the type's file.

### D6. Field visibility is part of the ABI (autonomous)

- Each field of an exported struct carries its visibility in `ExportedSymbol::Struct`, covered by the ABI hash and printed by `prism abi` (`private val id: Int`).
- Hidden fields stay in the ABI, as Rust's private fields stay in a crate's metadata: importers lay values out, copy, drop and compare them.
- Reachable definitions (`AbiTypeDefinition`) need no visibility. A file that reaches a type without importing its module cannot touch its members (`E4014`), and one that imports it reads the exported struct.
- Changing a field's visibility changes the ABI hash, so importers are re-checked. It changes no layout, so no slot contract and no type fingerprint changes, and hot reload keeps values (`hot-reload`).

**Compile-time cost.** One byte per exported field in Phase 1.

### D7. The library hides `Modifier`'s representation (autonomous)

`Modifier`'s fields (`count`, `first`, `second`) and `ModifierElements`' fields (`e0` to `e7`) become `private`. Only `/prism/compose/ui` reads them, through `append`, `element` and `then`, so the toolkit modules and programs build chains only through the modifier functions. The `ui-toolkit` follow-up "Field visibility, so that `ModifierElement`'s payloads and the element blocks are not readable or constructible by programs" is half done. `ModifierElement`'s variants stay constructible until enum variants can be hidden, which the follow-up keeps open.

The UI runtime reads chains by layout (`prism_ui_set_modifier` takes `&Modifier`), which visibility does not change.

**Compile-time cost.** None.

Rejected: **leaving the library unchanged here**. The library is the feature's first user, and the follow-up is listed in `ROADMAP.md`.

### D8. Formatting keeps field modifiers as written (autonomous)

`prism fmt` prints a field's modifier before `val`/`var`, in block and shorthand structs, and keeps an explicit `public`. The formatter's meaning check compares the new AST field.

**Compile-time cost.** None for compilation.

### D9. No new error code (autonomous)

- **`E4007` `VisibilityViolation`** is widened from `package` declarations to hidden fields: reads, writes, borrows, constructions, patterns and destructuring entries (D2 to D4).
- **`E2001`** covers misplaced or repeated field modifiers.

A hidden field is a visibility violation like a `package` declaration used from another package, so it shares that code.

### D10. Compile timing (autonomous)

`crates/prism-cli/tests/compile_timing.rs` gains the shape `field-visibility` at 256, 1,024 and 4,096: that many structs with a `private`, a `package` and an unmarked field, each constructed, read, written and matched by a named pattern in the struct's file, timing parse and check. Under `PRISM_ASSERT_TIMING=1`, the harness's linearity assertion holds (no stage of 5 ms or more grows over ×8 per ×4), and each stage grows on average by at most ×4.6 per ×4 (measured in task 5.2; the harness asserts that average for the borrow stage of its `LINEAR_BORROW` shapes only, which this shape does not stress).

### D11. Field visibility is checked where the code is written, once (autonomous, apply)

Added while applying. A field access is checked in the file whose source contains it, with that file's privileges, and nowhere else:

- **Generic bodies** are checked once at their declaration (`generics`), so a generic function or extension of a struct's own file reads and writes its private fields, and its instances, planned for and built in any program, re-check nothing. Code generic over `T` cannot name a field of whatever `T` is at all, so no instance can reach a field its body did not name.
- **Default values** are checked and generated in the declaring file (`mir`, default generators), so `public fun sized(text: String, b: Box<String> = Box(text.clone()))` builds a `Box` with a private field for callers in any file.
- **Bare names** in an extension body are rewritten to `this.name` only for a visible field (D5), and a bare name nothing else resolves that names a hidden field of the receiver is reported as that field (`E4007`, D3).
- **The error points at the access**, as `E4006` does for an unknown field: member accesses carry no span of their member name, so the caret is at the start of `a.balance`.

The record of the visibilities is the checker's own (`field_visibilities`, per struct with a marked field), not the shared type table, so typed trees, MIR, instance records and type fingerprints are unchanged.

**Compile-time cost.** None beyond D3: no instance or default re-checks anything.

Rejected: **re-checking instances with the caller's privileges**, which would re-check bodies per instance (against `generics`, checked once) and reject library generics that use their own types' fields.

**Baseline (task 1.2, on `d2cfc9cb`, idle machine, release build).** The `struct-fields` shape, in ms:

| program | lex | parse | abi | type | mir | borrow | compose | inst | lower | clif |
|---|---|---|---|---|---|---|---|---|---|---|
| struct-fields-256 | 0.0 | 0.1 | 0.0 | 0.3 | 0.2 | 0.8 | 0.0 | 0.0 | 0.6 | 2.4 |
| struct-fields-1024 | 0.1 | 0.3 | 0.0 | 1.1 | 0.7 | 3.3 | 0.2 | 0.0 | 2.4 | 6.2 |
| struct-fields-4096 | 0.5 | 1.0 | 0.1 | 4.3 | 3.4 | 13.6 | 0.7 | 0.2 | 9.6 | 28.3 |

**Measured (task 5.2, on `d4730b42` plus the shape, idle machine, release build).** In ms:

| program | lex | parse | abi | type | mir | borrow | compose | inst | lower | clif |
|---|---|---|---|---|---|---|---|---|---|---|
| struct-fields-256 | 0.0 | 0.1 | 0.0 | 0.3 | 0.2 | 0.9 | 0.1 | 0.0 | 0.7 | 2.4 |
| struct-fields-1024 | 0.1 | 0.3 | 0.0 | 1.1 | 0.7 | 3.4 | 0.2 | 0.0 | 2.4 | 6.1 |
| struct-fields-4096 | 0.5 | 1.0 | 0.1 | 4.4 | 3.6 | 14.1 | 0.8 | 0.2 | 9.7 | 28.6 |
| field-visibility-256 | 0.2 | 0.5 | 0.1 | 2.7 | 1.0 | 1.6 | 0.2 | 0.2 | 1.9 | 5.7 |
| field-visibility-1024 | 1.0 | 2.0 | 0.3 | 11.8 | 4.3 | 6.5 | 0.8 | 0.8 | 7.9 | 22.8 |
| field-visibility-4096 | 4.0 | 8.8 | 1.7 | 54.6 | 20.4 | 27.5 | 3.9 | 3.3 | 33.1 | 91.6 |

`struct-fields` matches the baseline of task 1.2 within noise (type 4.3 → 4.4 ms at 4,096, the other stages within 0.5 ms), so unmarked fields cost nothing measurable. `field-visibility`'s slowest step is ×4.7 per ×4 (MIR, 1,024 to 4,096), and its average growth per ×4 is at most ×4.52 (type ×4.50, MIR ×4.52, borrow ×4.15, clif ×4.01), within D10's ×4.6; the assertion run passed. The checks are one table lookup per field use.

## Risks / Trade-offs

- **[Hidden fields still cost layout]** Adding a private field changes importers' layouts, so it recompiles them. → This is the price of by-value structs across files, already the case for every field. Opaque handles would need boxes.
- **[Taking apart a value with hidden fields]** D4 lets another file move visible parts out of a struct whose hidden parts are then dropped. → A type with invariants across fields declares a drop hook, which forbids taking it apart, or keeps its non-`Copy` fields hidden.
- **[`E4007` where `E4006` was]** A misspelled access of a hidden field is reported as a visibility error. → The message names the field and its visibility.
- **[Packets landing in between]** → Task 1.1 re-checks that no open packet modifies the blocks this one adds or modifies, and rebases if needed.

## Migration Plan

Programs keep their meaning: unmarked fields are as visible as before. The library hides `Modifier`'s and `ModifierElements`' fields, which no example or fixture reads. The build cache is keyed by the compiler identity, so nothing cached is reused. Rollback: revert the commits.

## Open Questions

- Kotlin's `private set` (a `var` field readable everywhere and written only in its file), if library or application code wants it.
- Enum variant and payload visibility, for `ModifierElement`.
