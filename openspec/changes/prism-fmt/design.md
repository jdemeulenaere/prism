# Design: prism-fmt

## Context

See `proposal.md` for motivation. This change is item 13 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17; every decision the user might make differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

Current state:

- **Lexer** (`crates/prism-lexer/src/lexer.rs`): `skip_whitespace_and_comments` discards line comments and nested block comments; a `Token` carries its kind, its span and `is_preceded_by_newline`. Nothing records comments, so a tool that rewrites source has no way to keep them.
- **AST** (`crates/prism-ast`): every declaration, statement, expression, type, parameter, field, variant, branch and argument has a span. The tree does not record: parentheses (`parse_group` returns the inner expression with the span widened to the parentheses; type parentheses `(Int)` vanish entirely), literal spelling (`Literal::Int(i64)` holds the value, `String` the decoded text), an explicit `private` (it is the default `Visibility::Private`), the struct shorthand form (`struct Id(val id: Int)` and `struct Id { val id: Int }` give the same `StructDecl`), whether a shorthand field wrote `val`, the import separator (`/` or `.`), separators (`;`, `,`), trailing commas, empty `()` before a trailing lambda, and comments. An unbraced branch or loop body that is an assignment is a `Block` whose `close_brace` equals its span; a braced block starts with `{`.
- **Newline significance** (`syntax`, "Newline Before a Binary Operator Ends the Expression"): outside parentheses and brackets, a line break ends an expression unless the next token is `.`, `?.`, `&&`, `||` or `?:`; a call's `(`, an index's `[` and a trailing lambda's `{` must be on the line of what precedes them; `return` takes a value only on its own line. A line break after a binary operator, after `=`, `->` or `(`, and inside parentheses and brackets never changes the parse.
- **CLI** (`crates/prism-cli/src/main.rs`): one `match` over commands; `check` goes through `workspace.rs`, the file-local commands (`dump-ast`, `tokens`) parse one file. `format_parse_error` renders `E2001`/`E2002` as `prism check` does.
- **Corpus**: about 1,170 `.prism` files under `crates/prism-cli/tests/fixtures/` and `examples/`, many deliberately malformed or oddly laid out to exercise the parser.

## Goals / Non-Goals

**Goals:**
- One canonical, width-aware layout for the whole current grammar, with no options.
- No lost comment, no changed meaning, idempotent output, all checked by tests over the whole corpus and, for meaning and comments, on every run.
- No cost to compilation: the comment-collecting lexer mode is opt-in.

**Non-Goals:**
- Formatting files with syntax errors or ranges of lines, which need parser error recovery.
- Import sorting, trailing-comment alignment, re-indenting block-comment bodies (deferred, see the proposal).
- Reformatting the test fixtures: they deliberately keep unusual layouts that the parser tests rely on.

## Decisions

### D1. An AST printer over a document algebra (autonomous)

The formatter parses the file, then prints the syntax tree into a document of the Wadler/Prettier algebra: text, `line` (a space, or a newline when its group breaks), `softline` (nothing, or a newline), hard line, indentation, `group` (printed flat when it fits in the remaining width, otherwise broken) and `if_break` (text only in a broken group, used for trailing commas). A renderer lays the document out at width 100 with Prettier's `fits` rule (a group is flat when its content, and what follows it up to the next possible line break, fits). A group that contains a hard line or a line comment is broken. Breaks are placed only where `syntax`'s newline rules keep the parse (after an operator, `=`, `->` or `(`, before `.`, `?.` and `?:`, inside parentheses and brackets), so any layout the renderer picks parses as the input.

The crate is `crates/prism-formatter` (`prism_formatter::format_source(&str) -> Result<String, FormatError>`), depending on `prism-ast`, `prism-lexer` and `prism-parser` only; `prism-cli` calls it.

Rejected: a token-stream reindenter (clang-format, early gofmt style), which can fix spacing and indentation but cannot decide where to break a long call or join a short one, and would keep each author's line breaks; a canonical layout without width (always one argument per line, or never breaking), which produces either very tall or very long code. Rejected: printing from the typed tree or MIR, which have desugared away what the user wrote.

### D2. Comments come from an opt-in lexer mode (autonomous)

`Lexer::tokenize_with_comments(self) -> (Vec<Token>, Vec<Comment>)`, with `Comment { kind: CommentKind::{Line, Block}, span }` (the text is the span's slice of the source). The lexer gets an `Option<Vec<Comment>>` that `skip_whitespace_and_comments` pushes to when it is `Some`; `Lexer::new` and `tokenize_all` leave it `None`, so compilation pays one untaken branch per comment. The formatter lexes once in this mode and hands the tokens to `Parser::new`, so the tree and the comments come from the same scan.

Rejected: trivia on every token (Roslyn, rust-analyzer), which grows every token and the parser's copies for the benefit of one tool and would be the right choice only for a lossless syntax tree (see D13); re-scanning the gaps between tokens in the formatter, which duplicates the nested block-comment rules; comments as AST nodes, which every later stage (ABI body hashing, the type checker) would have to skip.

### D3. Comment placement by list items, with a fallback at nodes (autonomous)

Comments are attached while printing, in source order, by a cursor over the comment list that only moves forward, so the output keeps the comments' order by construction:

- **Leading**: before each item of a list (top-level declaration, struct field, enum variant, statement, block tail, `when` branch, parameter, argument), the comments that start before the item are printed on their own lines, at the item's indentation. A block comment followed by the item on its line stays inline (`/* two */ val b = 2`).
- **Trailing**: after an item and its separator, a line comment, or a block comment ending its line, that starts on the line where the item ends is printed after one space on the same line.
- **Dangling**: comments left before a list's closing `}` or `)` go on their own lines before it.
- **In comma lists** (parameters, arguments), the comments after an item on its line, and those left inside it, follow the item's comma, where the next run finds them in the same place; this keeps `a /* x */, b` stable as `a, /* x */ b`.
- **Before tokens**: a comment between an operand and its binary operator, or between an assignment target and its `=`, stays before the operator; a comment after an operator on its line stays there (`1 + // plus`); a comment before a `.` of a call chain breaks the chain there, so it stays before that member.
- **Fallback**: a comment elsewhere (inside a type, between modifiers) is printed before the next node that starts after it: a block comment inline, followed by a space, and a line comment after one space and followed by a forced line break, the code after it continuing one level deeper than the line the comment ends. Comments left inside an item when it ends are flushed at its end.
- **Before a block's `{`**: when one of them ends its line, the comments before a `{` move inside the block, before its first item, so the `{` stays on its header's line.

A line comment is a document primitive that ends the output line: anything printed after it starts a new line, and its groups are broken. That keeps the comment's meaning (the rest of the line is a comment) without a deferred "line suffix", whose reordering against a later block comment on the same line would break the order guarantee.

Rejected: Prettier's full attachment (enclosing, preceding and following node for every comment, with per-node handlers), which gives finer placement in rare positions at a much larger cost; dropping comments the attachment does not understand.

### D4. Meaning is verified on every run (autonomous)

After printing, `format_source` lexes and parses its own output, erases every span of both trees (a mutable walk over the whole AST, exhaustive so that a new AST field must be handled) and compares them, and compares the comment texts in order (line comments with trailing whitespace removed). On any difference it returns `FormatError::Internal`, and `prism fmt` leaves the file untouched and reports "internal error: ... this is a bug in prism fmt". The extra parse costs about as much as the first.

This is Black's AST safety check. It turns every printer bug, including a line break that the newline rules give a meaning, into a refusal instead of a silent change. The corpus test additionally checks idempotency (`fmt(fmt(x)) == fmt(x)`), which is not checked at run time because it would double the cost for a property that does not affect correctness.

An internal error is not a diagnostic of the user's source, so it takes no code from the `diagnostics` catalog (like a crash, which the catalog does not list either).

Rejected: comparing token streams minus trivia, which cannot work once the formatter drops separators, trailing commas and redundant `()`; trusting the printer and testing only.

### D5. Written forms the tree does not record are read from the tokens (autonomous)

The printer keeps the token list of the lexed file, sorted by offset, and reads the written form back where the tree lost it:

- **Literals** are printed as their source text (the first token at the expression's start, past any `(`; for the `i64::MIN` literal, `-` and the number), so `0xFF`, `1_000`, `1e3` and escapes survive.
- **Parentheses**: an expression with a left operand (binary, call, member access, index) was parenthesized when its span starts before that operand; any other expression when its first token is `(`. Such an expression is printed in one pair of parentheses (nested pairs collapse). The printer also adds parentheses wherever precedence requires them, as a safety net that the verification makes visible if it is ever needed.
- **`private`**: kept when the declaration's first token is `private`.
- **Struct form**: the shorthand form is kept when the declaration does not end in `}`; a shorthand field's `val`/`var` when the field's first token is one.
- **Braced bodies**: a block is braced when its first token is `{`.
- **One-line struct and enum bodies** (D9): when the `{` and the `}` are on the same source line.

Type parentheses are not recorded at all (`(Int)` is `Int` in the tree), so only the ones meaning needs are printed: around a function type, a reference or a nullable type made nullable (`(() -> Unit)?`, `(&Int)?`, `(Int?)?`).

Normalized, since the tree is the same either way and Kotlin's conventions prefer one form: separators, trailing commas (D7), empty `()` before a trailing lambda (`foo() { }` is `foo {}`, IntelliJ's "remove unnecessary parentheses"), and the import separator, `/` as in every spec example.

Rejected: removing redundant parentheses (Prettier), which second-guesses parentheses written for clarity (`(a && b) || c`); always printing `private`, or always dropping it (Kotlin drops redundant modifiers, but removing words is beyond layout); converting shorthand structs to block form.

### D6. Style: the Kotlin coding conventions (autonomous)

Four-space indentation, also for continuation lines; width 100; spacing as in the `formatting` spec. Specific choices:

- **Breaking lists** (parameters, arguments, shorthand fields, enum payloads): flat when they fit, else one item per line with a trailing comma and the closer on its own line, which is the layout ktfmt and IntelliJ produce. Type-parameter and type-argument lists and types never break.
- **Signatures**: `[visibility] [compose] [mut | consuming] fun [<T : Copy>] [Receiver.]name(params)[: Return]`, with ` {` or ` =` on the closing line.
- **The right side of `=`, and a `when` branch body after `->`**: a call, closure, block, `when` or an `if` with a braced branch starts on the same line and breaks inside itself (`val x = foo(\n    a,\n)`); anything else moves to the next line, indented, when it does not fit. This is Prettier's "break after operator" layout, limited to expressions that have no better break of their own.
- **Binary operators**: a chain of operators of one precedence is one group, breaking after each operator (`a +\n    b`), the Kotlin convention, which also keeps the parse: a line starting with `+` or `-` would start a new statement. `?:` is the exception, breaking before (`\n    ?: b`), as the conventions recommend and the newline rule allows. `&&` and `||` break after, like the other binary operators.
- **Call chains**: a chain with at least two calls (`a.b().c()`) is a group that breaks before each `.` or `?.` that starts a call segment, the head (the receiver and its plain field accesses) staying on the first line; a chain with one call breaks inside its arguments instead (`user.rename(\n    ...\n)`).
- **`if`**: `if (c) {` / `} else if (d) {` / `} else {` for braced branches; unbraced branches stay on one line when they fit (`if (c) a else b`), otherwise each branch goes to its own indented line after `if (c)` and `else`. Conditions break only inside their own expression, as Kotlin's `if (a &&\n    b) {`.
- **`when`**: always one branch per line, even a single branch, as in Kotlin.
- **Blocks** are always broken; closures with a single expression stay on one line when they fit.
- **Trailing comments** get one space before them, no column alignment.

Rejected: rustfmt's style (width 100 but trailing commas everywhere and a different chain rule); a width of 120 (IntelliJ's default) or 80 (Prettier's default).

### D7. Trailing commas only in broken lists (autonomous)

A broken parameter, argument, shorthand-field or enum-payload list gets a trailing comma after its last item; a flat one has none. The grammar accepts a trailing comma in all of them, and Kotlin's conventions recommend it at declaration sites because it keeps diffs to one line. Call-site arguments get it too, for one rule everywhere (ktfmt does the same). No trailing comma is printed before a trailing lambda, which is outside the parentheses.

Rejected: never printing trailing commas (Kotlin before 1.4); keeping each author's commas, which is not canonical.

### D8. Blank lines: at most one, kept where written (autonomous)

Between two items of a vertical list (declarations, members, statements, branches) and around comments, one or more blank lines in the input become one; there are none after `{`, before `}` or at the start of the file. Exactly one blank line follows the package declaration and the import block, and separates two top-level declarations unless both are `val`s (IntelliJ's Kotlin defaults: one blank line around functions and classes, none required between properties). The file ends with one newline; a file with nothing in it formats to empty. Blank lines inside parameter and argument lists are dropped.

Rejected: keeping adjacent top-level declarations adjacent (gofmt), which leaves functions and structs run together; deciding by whether a declaration spans several lines, which depends on the width and would not be idempotent.

### D9. Struct and enum bodies and closures keep the author's one-line choice (autonomous)

A struct or enum body written on one line (`enum Color { Red, Green, Blue }`) and without comments is a group: flat, with `, ` separators, when it fits, otherwise one member per line without separators. A body written over several lines is always one member per line. Likewise a closure with a single statement or expression stays on one line only when it was written on one line and fits: `remember { mutableStateOf(0) }` and `onClick = { count += 1 }` stay compact, while a layout closure written expanded (`Column {\n    Text("a")\n}`) stays expanded. This is Prettier's rule for object literals and gofmt's for function literals: the author decides between a compact and an expanded form, and the formatter never joins what was written expanded. The choice is idempotent, since the output's line structure is what the next run reads. A `when` has no such choice (D6), and a closure with several items is always broken.

Rejected: always one member per line, which turns `enum Bit { Zero, One }` into four lines; always joining when it fits, which collapses carefully laid-out enums and turns every compose container with one child into a one-liner.

### D10. Files with syntax errors are refused (autonomous)

The formatter needs a complete tree, and the parser stops at its first error. A file that does not parse is reported with `format_parse_error`, as `prism check` reports it, left untouched, and makes the command exit non-zero; the other files are still formatted. Rejected: formatting up to the error (gofmt and rustfmt refuse too); a token-level fallback for broken files, which would give two formatters.

### D11. CLI surface (autonomous)

`prism fmt [--check] <path>...`:

- Paths are files or directories; directories are walked recursively in sorted order for `*.prism` files, skipping directories whose name starts with `.` (such as `.git` and editor or tool directories) and not following symbolic links to directories (no cycles). A file named explicitly is formatted whatever its extension. `-` is standard input to standard output (gofmt's and Black's convention).
- Without `--check`, changed files are rewritten and their paths printed (`gofmt -l -w`); unchanged files are not written, so their timestamps do not change.
- With `--check`, nothing is written, the paths of files that would change are printed, and the exit status is 1 if there is any (`rustfmt --check`, `gofmt -l` in CI).
- Exit status 1 also for syntax errors, read or write errors (`E1000`, as `prism check` reports I/O) and internal errors.
- No configuration file and no options: a formatter configuration file would be a build file, against the zero-build-file invariant, and one style is the point of a canonical formatter (gofmt).

The directory walk and the per-file loop live in `prism-formatter` (`prism_formatter::files`), so that `main.rs` only parses arguments, renders diagnostics and sets the exit status.

### D12. Spec placement (autonomous)

A new `formatting` capability holds the command, the layout and the guarantees. The comment-collecting lexer mode is a lexer behaviour, so it goes to `syntax`, beside "Lexical Tokenization and Escape Sequences". Formatting scenarios are tested by golden files in `crates/prism-formatter/tests/golden/`, named after their scenario slugs, and by CLI tests in `crates/prism-cli/tests/`, since the `prism check` fixture harness cannot express an expected output.

### D13. No lossless syntax tree yet (autonomous)

A lossless concrete syntax tree (rust-analyzer's rowan, Roslyn) would make the formatter, and later the LSP, simpler and more exact: every token, comment and parenthesis would be in the tree. It is the long-term design for the LSP of Phase 5, but it means replacing the parser's output type, which every stage consumes. This change keeps the AST and recovers what it lacks from the token list (D5) and the comment list (D2), which a later lossless tree would replace without changing the formatter's behaviour or its tests.

## Risks / Trade-offs

- [A comment in an unusual position is moved to the next node or to the end of its item, and in rare cases forces a line break the author did not want] → the order and text of comments are guaranteed and verified; placement is tested on the common positions (D3).
- [A layout that the newline rules give another meaning would silently change code] → the run-time verification refuses it (D4), and the corpus test exercises about 1,170 files.
- [The verification doubles the cost of formatting] → formatting a file is a lex and two parses, milliseconds for any realistic file; idempotency is checked only in tests.
- [Long string literals and long names still exceed 100 columns] → there is no break that keeps the meaning; this is the same in every formatter.
- [Multi-line block comments keep their inner lines' indentation when the comment moves] → harmless and idempotent; re-indenting them is a follow-up.

## Migration Plan

No external users. `examples/` is reformatted in this change; the test fixtures are not (non-goal). Rollback is a revert.

## Open Questions

- Whether `prism check` should later warn about unformatted files (as `cargo fmt --check` in CI does) can be decided once the formatter has been used for a while.
