# Proposal: string-templates

## Why

After `runtime-strings-and-drops`, a program can build text only by concatenation and explicit conversion (`"Count: " + count.toString()`), which allocates once per `+` and once per `toString()`. Kotlin code, and Phase 4's UI code above all (`Text("Count: $count")`), writes string templates instead, and a composable rebuilds its texts on every recomposition, so a template must cost one allocation, not one per part. This change, item 19 of "Then: the road to Milestone 2" in `ROADMAP.md`, adds Kotlin's string templates, following the design `runtime-strings-and-drops` recommended (its D20) and settling what it left open: how templates are lexed, which parts have a text form, what a nullable part does, how the parts are borrowed, and how the formatter prints them.

## What Changes

- **Template syntax.** A string literal may hold `$name`, which inserts the value of `name` (an identifier, or `this`), and `${expression}`, which inserts the value of any expression, as in Kotlin: `"Count: $count"`, `"${a + b} items"`. A `$` followed by anything else (a space, a digit, `"`, the end of the text) is a literal `$`, and the new escape `\$` writes a literal `$` anywhere, also in a `Char` literal. `$if` and the other keywords except `this` are syntax errors (`E2001`). An expression part may hold any expression, including nested string literals and templates, closures and `when` with their braces, comments and newlines. The text between parts still cannot hold a raw newline.
- **Lexing.** The lexer emits a template as a flat run of tokens: a template start, text runs, `$name` references, `${` and its matching `}` around the ordinary tokens of each expression, and a template end. It tracks nesting with a mode stack and counts braces inside `${}`. A literal without any part stays a single `String` token, as today.
- **Parts and their text.** Each part converts to text without being moved:
  - a `String` is read in place, borrowed until the result is built;
  - an `Int`, `Double`, `Bool` or `Char` gives the text `print` writes;
  - any other type calls its `toString()`, as `.toString()` would resolve it (a field, or an extension, visible or imported), provided that it takes a shared receiver, needs no arguments and returns `String`.
  Parts are read through `&`, `&mut` and `Heap<T>` layers, as `print` reads its argument.
- **Nullable parts are rejected.** Kotlin prints `null`. Prism rejects a nullable part with help suggesting a fallback (`${name ?: "anonymous"}`), so a UI never shows "Hello null" by accident. Accepting them later would not break any program; rejecting them later would.
- **A new diagnostic, `E4026` (`NoTextForm`).** It covers a part without a text form: a nullable, `Unit`, a function value, a type parameter, or a struct or enum without a usable `toString()`. Its message says why, and its help names the fix: a fallback for `null`, the extension to declare (`fun User.toString(): String`), `${u.name}` when `$u.name` was meant, or an explicit `${c.toString()}` call for a `mut` or `consuming` `toString()`.
- **One allocation per template.** The typed tree has a template node and MIR a template rvalue whose parts are text, borrowed strings and scalar values. The backend lowers it to one call of a runtime function over a table of the parts. The runtime formats the scalars itself, sizes the result, and allocates it once with exactly that capacity. An empty result allocates nothing. Parts are evaluated left to right. Only a part whose type needs a `toString()` call allocates a temporary of its own, which is dropped at the end of the statement.
- **Formatting.** `prism fmt` prints a template's text and `$name` parts exactly as written, escapes included, formats each `${...}` expression on one line, and never breaks a template across lines. A template whose expression parts hold a comment, a newline or a construct that cannot be printed on one line is printed exactly as written.
- **Fixtures.** Every runtime scenario is an executable fixture, run under `--check-allocations`, so a leaked or doubly freed part or result fails the test suite.
- Not in scope: raw and multi-line strings (`"""`, `trimIndent`), format specifiers (`"%.2f"`), `print`/`println` of types with a `toString()` extension, and templates evaluated at compile time. Each becomes a `ROADMAP.md` follow-up (`design.md`, D1, D7 and D16).

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `syntax`: template literals, the `\$` escape, the template tokens and the template expression in the tree.
- `type-system`: the type of a template, the text form of each part type, and `E4026`.
- `references`: template parts are borrowed in place, never moved, until the template is built.
- `typed-tree`: the template node and its parts.
- `mir`: the template rvalue, its operands and evaluation order.
- `code-generation`: templates as one runtime call over a parts table, with one allocation.
- `execution`: the text of a template at run time.
- `formatting`: how templates are printed.

## Impact

- `crates/prism-lexer`: new token kinds and a mode stack in `lexer.rs` and `token.rs`, and the `\$` escape in `lex_escape`.
- `crates/prism-ast`, `crates/prism-parser`: `ExprKind::Template` and `TemplatePart`, and `parse_template`, called from `parse_prefix`.
- `crates/prism-abi`: the span-free expression hash (`hash_expr_span_free` in `signature.rs`) covers templates.
- `crates/prism-formatter`: a `flat` document primitive, template printing in `printer/expressions.rs`, the tree comparison in `verify.rs`, the corpus variants, and golden files.
- `crates/prism-typed-tree`, `crates/prism-typechecker`: `ExpressionKind::Template`, `check_template_part` (which reuses the member-call resolution of `infer_method_call`), and `TypeError::NoTextForm` (`E4026`).
- `crates/prism-mir`, `crates/prism-borrowck`: `Rvalue::Template`, built in `build/comparison.rs` next to `lower_binary`, and the uses, conflicts and loan flow of its operands.
- `crates/prism-runtime`, `crates/prism-codegen`: `prism_rt_string_template` and its part table in `string.rs`, and the lowering in `lower.rs`.
- `crates/prism-cli`: fixtures under `syntax/`, `type-system/`, `references/`, `typed-tree/`, `mir/`, `code-generation/` and `execution/`, and `examples/run/main.prism`.
- Specs: the deltas above, and a new `E4026` row in the catalog in `openspec/specs/diagnostics/spec.md`.
- Builds on `runtime-strings-and-drops` (strings, scalar `toString()`, the allocator and `--check-allocations`), `payload-patterns` (which takes `E4024` and `E4025`) and `heap-boxes` (reading through boxes). All three must be archived first.
