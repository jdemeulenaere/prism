# Tasks

## 1. Lexer Verification and Hardening (`prism-lexer`)

- [x] 1.1 Verify and expand lexical tokenization for comments (single-line and deeply nested `/* ... */`), ensuring unclosed block comments produce clean `TokenKind::Error` with exact spans; verify with `cargo test -p prism-lexer --test lexer_tests test_nested_block_comments`.
- [x] 1.2 Verify string and char literal escapes, unicode code points (`\u{1F680}` and `\u0041`), hex lookahead validation before closing quotes, and rejection of unescaped raw newlines in single-line strings; verify with `cargo test -p prism-lexer --test lexer_tests test_utf8_strings_and_chars`.
- [x] 1.3 Verify numeric literals (integers, hex `0x`, binary `0b`, floats with decimals and scientific notation `e`/`E`, digit separators `_`, and range lookahead disambiguation `4..10`); verify with `cargo test -p prism-lexer --test lexer_tests test_scientific_notation_floats` and `test_numbers_and_ranges`.
- [x] 1.4 Verify newline tracking (`is_preceded_by_newline`) across all token kinds and whitespace permutations; verify with a dedicated unit test in `crates/prism-lexer/tests/lexer_tests.rs`.

## 2. Pratt Parser Operator Precedence and Grammar Verification (`prism-parser`)

- [x] 2.1 Verify operator precedence table across arithmetic, logical, comparison, Elvis (`?:`), range (`..`), and non-associative assignment (`=`) rejecting chained assignment (`a = b = c`), and verifying Elvis precedence (`age ?: 0 >= 18` and `base ?: offset + 10`); verify with `cargo test -p prism-parser --test parser_tests test_elvis_and_safe_nav` and a new precedence test in `crates/prism-parser/tests/parser_tests.rs`.
- [x] 2.2 Implement and verify Pratt `LParen` newline suppression so leading parenthesis expressions on new lines are parsed as independent statements rather than function calls; verify with a dedicated unit test in `crates/prism-parser/tests/parser_tests.rs`.
- [x] 2.3 Implement and verify trailing lambda syntax parsing (`Column { ... }`, `Button(id) { ... }`) and multi-statement closure bodies with parameters (`{ item -> val s = item * item; println(s) }`); verify with `cargo test -p prism-parser --test parser_tests test_general_trailing_lambdas`.
- [x] 2.4 Require parentheses on `if (...)` and `when (...)` conditions to eliminate trailing lambda grammar conflicts in `parse_if` and `parse_when`; verify with `cargo test -p prism-parser --test parser_tests test_when_inside_block`.
- [x] 2.5 Verify newline-sensitive return statement termination (`return\n expr` vs `return expr\n`) and semicolon-free statement separation; verify with `cargo test -p prism-parser --test parser_tests test_newline_aware_return`.

## 3. Declarations, Contextual Modifiers, and Invariant 4 Enforcement

- [x] 3.1 Verify top-level declaration parsing (`fun`, `compose fun`, `struct`, `enum`, `val`, `var`) and default `private` visibility enforcement with explicit `package` and `public`; verify with `cargo test -p prism-parser --test parser_tests test_parse_math_sample` and `test_parse_compose_function`.
- [x] 3.2 Allow `compose` as a contextual identifier in member access (`pipeline.compose(step)`) and parameter declarations; verify with a dedicated test in `crates/prism-parser/tests/parser_tests.rs`.
- [x] 3.3 Enforce Invariant 4 by validating struct fields in `parse_struct` to reject borrowed references (`&T`, `&mut T`); verify with a negative test in `crates/prism-parser/tests/parser_tests.rs`.
- [x] 3.4 Add negative error diagnostic tests in `crates/prism-parser/tests/parser_tests.rs` asserting specific `ParseError` diagnostics and spans on malformed syntax without compiler panics.

## 4. End-to-End Front-End Integration Suite

- [x] 4.1 Run full workspace tests across `prism-lexer`, `prism-parser`, `prism-ast`, `prism-abi`, and `prism-resolver` using `cargo test`; verify that all tests pass cleanly with 0 failures.
