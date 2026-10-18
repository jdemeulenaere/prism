# Tasks

The `formatting` scenarios are tested by golden files in `crates/prism-formatter/tests/golden/` (`<scenario-slug>[--<form>].prism` and its expected output `<scenario-slug>[--<form>].expected.prism`; `PRISM_BLESS=1` writes the expected files, each read against its scenario's THEN before committing) and by CLI tests in `crates/prism-cli/tests/fmt_cli_tests.rs` named after their scenarios (D12).

## 1. Comment-collecting lexer (D2)

- [x] 1.1 In `prism-lexer`, add `Comment { kind, span }`, `CommentKind::{Line, Block}` and `Lexer::tokenize_with_comments`, recording comments in `skip_whitespace_and_comments` only when collection is on; `tokenize_all` and `next_token` collect nothing. Verify with lexer tests for both scenarios of "Comments Are Available to Tools" (the token streams of both modes compared equal, kinds, spans and newline flags included), and that `cargo test -p prism-lexer` passes.

## 2. Document algebra and renderer (D1)

- [ ] 2.1 Create `crates/prism-formatter` (workspace member, depending on `prism-ast`, `prism-lexer`, `prism-parser`) with the document type (text, line, softline, hard line, indent, group, if-break, line comment), break propagation of hard lines and line comments to enclosing groups, and an iterative renderer with Prettier's `fits` rule at width 100 and four-space indentation that trims trailing whitespace. Verify with unit tests: a group that fits stays flat, one that does not breaks, the rest of the line counts towards fitting, an if-break prints only when broken, and a line comment ends its line and breaks its groups.

## 3. Printer (D3, D5, D6, D7, D8, D9)

- [ ] 3.1 Print the file structure, declarations and types: package, imports with `/`, functions (modifiers, type parameters with bounds, receivers, parameters with defaults, return types, block and expression bodies, bodiless declarations), structs in both forms with markers, enums with payloads, top-level `val`s, every type form with the parentheses meaning needs, blank lines (D8) and one-line bodies (D9). Verify with golden files for every scenario of "Spacing", "Breaking Lists", "Struct and Enum Bodies", "File Structure" and "Blank Lines Between Items", and for "Struct forms and private kept".
- [ ] 3.2 Print statements and expressions: locals, assignments, compound assignments, `*r = v`, `while`, `do`/`while`, `for`, blocks, `if`/`else` (braced and unbraced), `when` with every pattern form, `return`, `break`, `continue`, closures, calls with named arguments, type arguments and trailing lambdas, member chains, safe navigation, index, unary and borrow operators, binary operators with the D6 breaking rules, literals and parentheses from the tokens (D5). Verify with golden files for every scenario of "Indentation and Line Width", "Breaking Chains and Operators", "Breaking After Equals and Arrows", "One Item Per Line" and "Blocks and Closures", and for "Literals and parentheses kept".
- [ ] 3.3 Attach comments (D3): leading, trailing and dangling comments in every list, the node fallback and the end-of-item flush. Verify with golden files for every scenario of "Comment Placement" and "Comment Text", and for comments in each list kind (declarations, members, statements, branches, parameters, arguments) and inside expressions and types.

## 4. Verification and corpus (D4)

- [ ] 4.1 In `format_source`, re-parse the output and compare the span-erased trees and the comment texts; return `FormatError::Syntax` for input that does not parse and `FormatError::Internal` on a mismatch. Verify with a unit test that the comparison detects a changed tree and a lost comment, and a corpus test over every `.prism` file under `crates/prism-cli/tests/fixtures/`, `examples/` and the golden directory: each file that parses formats without an internal error (scenario "Every fixture and example keeps its meaning") and formats to itself when formatted again ("Formatting formatted code"), each file that does not parse gives `FormatError::Syntax`, and every expected golden file is its own formatting.

## 5. Command (D10, D11)

- [ ] 5.1 Add `prism_formatter::files` (collecting files from paths per D11, and formatting or checking one file) and the `fmt` command in `crates/prism-cli/src/main.rs` with its usage line, rendering syntax errors with `format_parse_error` and I/O errors as `E1000`. Verify with CLI tests for every scenario of "Format Command", "Formatting Directories", "Formatting Standard Input", "Check Mode" and "Files With Syntax Errors Are Not Formatted".

## 6. Integration

- [ ] 6.1 Run `prism fmt examples/` and commit the result after reading the diff. Run the apply guidance in `openspec/config.yaml`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (no new warnings), `cargo test --workspace`, `prism check` on every file under `examples/` (only `examples/demo/ui` may fail), `prism fmt --check examples/`, and `openspec validate --all --strict`. Verify all pass.

## Workflow follow-up

- On archive, promote D1 to D11 and D13 into a `## Design Rationale` section of the new `formatting` spec, and D2 into `syntax`'s. Add the `formatting` spec to the reference list in `AGENTS.md`.
- In `ROADMAP.md`, tick item 13 of "Then: the road to Milestone 2" and the Phase 1 item "Code formatting command (`prism fmt`)", and add as unchecked items, each naming `prism-fmt`:
  - import sorting;
  - aligning consecutive trailing comments;
  - re-indenting the inner lines of multi-line block comments;
  - formatting files with syntax errors and line ranges, after parser error recovery;
  - a lossless syntax tree for the formatter and the LSP (D13), with editor format-on-save in Phase 5;
  - whether `prism check` warns about unformatted files.
