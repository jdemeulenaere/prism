# Proposal: prism-fmt

## Why

Prism has no formatter, so every fixture, example and future library file carries its author's layout, and every review spends attention on whitespace. A canonical layout, applied by one command, is part of the Kotlin-like ergonomics the language promises, and it has to exist before the compose and standard-library code of Milestone 2 is written, so that code is formatted from its first line. This is item 13 of "Then: the road to Milestone 2" in `ROADMAP.md`, and the Phase 1 item "Code formatting command (`prism fmt`)". The AST is now stable enough (generics, extension functions, compound assignment, write-through, loops all landed) to cover the whole grammar at once.

## What Changes

- **A `prism fmt` command.** `prism fmt <path>...` rewrites each named file, and every `.prism` file under each named directory (recursively, skipping hidden directories), in place with the canonical layout, and lists the files it changed. `prism fmt -` formats standard input to standard output. `--check` writes nothing, lists the files that would change, and exits non-zero if there is any. There is no configuration file and no style option: one layout for every Prism file, in the spirit of the zero-build-file invariant.
- **The canonical layout follows the Kotlin coding conventions.** Four-space indentation and a line width of 100 characters; width-aware breaking (a list that does not fit is broken one item per line, with a trailing comma; a long chain of calls breaks before each `.`; a long binary expression breaks after its operator, and before `?:`); one statement per line with `;` separators dropped; spaces around binary operators and after commas; `name: Type`, `<T : Copy>` and `struct Point : Copy`; blocks always multi-line, a closure on one line when its single expression fits; one `when` branch per line; at most one blank line kept between items; one newline at the end of the file.
- **Comments are preserved.** A new comment-collecting mode of the lexer returns every comment with its span, beside the unchanged token stream; normal compilation never collects comments. Comments on their own line stay on their own line before what follows them, comments at the end of a line stay at the end of that line, and comment text is kept verbatim (except trailing whitespace).
- **Formatting never changes meaning.** Every run re-parses its output and compares the syntax tree (ignoring source positions) and the sequence of comments with those of the input; on any difference the file is left untouched and an internal error is reported. Formatting is idempotent: formatting formatted code changes nothing. Both properties are tested over every `.prism` fixture and example in the repository.
- **Files with syntax errors are not formatted.** `prism fmt` reports the same `E2001`/`E2002` diagnostic as `prism check`, leaves the file untouched, still formats the other files, and exits non-zero.
- **Written choices that the tree does not record are kept:** literal spellings (`0xFF`, `1_000`, `1e3`, escapes), parentheses around expressions, an explicit `private`, the shorthand or block form of a struct, braced or unbraced bodies, and a struct or enum body written on one line (when it still fits). Normalized: whitespace, line breaks, separators between members and branches, trailing commas, empty `()` before a trailing lambda, and `/` as the import path separator.
- **Not in scope (deferred):** sorting imports; aligning consecutive trailing comments; re-indenting the inner lines of multi-line block comments; formatting a range of lines or a file with syntax errors (needs error recovery in the parser); editor and LSP integration (Phase 5).

## Capabilities

### New Capabilities
- `formatting`: the `prism fmt` command, its canonical layout, comment and blank-line preservation, meaning preservation and idempotency, and its refusal of files with syntax errors.

### Modified Capabilities
- `syntax`: a comment-collecting lexer mode that yields the same tokens as normal lexing plus every comment with its kind and span.

## Impact

- `crates/prism-lexer`: `Comment`, `CommentKind` and `Lexer::tokenize_with_comments`; `tokenize_all` and `next_token` are unchanged and collect nothing.
- `crates/prism-formatter` (new): a document algebra with a width-aware renderer, the printer for every AST node, comment attachment, and the meaning check (span-erased tree comparison and comment comparison). Depends on `prism-ast`, `prism-lexer` and `prism-parser` only: formatting is per file and resolves no imports, per the single-file compilation unit invariant.
- `crates/prism-cli`: the `fmt` command in `main.rs` and its usage text; CLI tests for in-place formatting, directories, standard input, `--check` and syntax-error refusal.
- `examples/`: reformatted with `prism fmt`.
- `ROADMAP.md`, `docs/decisions-for-review.md`: the decisions below, and on archive the deferred items.
