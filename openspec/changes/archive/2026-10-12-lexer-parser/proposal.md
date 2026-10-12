# Proposal

## Why

A resilient, fast streaming tokenizer and Pratt parser are the essential front-end foundation for the Prism compiler (Phase 1). Formalizing the lexical and grammatical specifications—including nested comments, unicode escapes, Pratt precedence, trailing lambdas, closures, ASI/newline rules, and architectural invariants—ensures unambiguous parsing and prevents compiler bugs before Phase 2 type checking and code generation.

## What Changes

- **Formalize Lexical Specifications**: Define requirements and testable scenarios for tokenization, nested block comments (`/* ... /* ... */ ... */`), string/char escape sequences including unicode code points (`\u{...}`), rejection of unescaped raw newlines in single-line strings, hex/binary/scientific number literals, and whitespace/newline tracking.
- **Formalize Pratt Expression Grammar & Operator Precedence**: Specify operator precedence rules (from non-associative assignment up to call and postfix), ordering Elvis (`?:`) strictly above comparison/equality and below arithmetic, and rejecting chained assignments (`a = b = c`).
- **ASI and Newline Delimitation**: Semicolon-free statement termination with newline guards in Pratt precedence suppressing accidental function calls (`LParen` on newlines) and unary/binary operator misinterpretation across lines.
- **Trailing Lambdas and Multi-Statement Closures**: Define grammar rules for trailing closure/lambda blocks (`Button(onClick) { ... }`) and multi-statement closure bodies with parameters (`{ item -> val x = item; println(x) }`).
- **Mandatory Parentheses for Control Flow**: Require parentheses around `if (...)` conditions and `when (...)` subjects to eliminate LL(1) Pratt conflicts between condition expressions and trailing lambdas.
- **Contextual Modifier Handling**: Treat keywords like `compose` as soft/contextual keywords so they remain usable as identifiers in member access (`pipeline.compose()`) and parameter names.
- **Enforce Invariant 4 in Structs**: Disallow borrowed reference types (`&T`, `&mut T`) in struct fields at the parser level, guaranteeing zero lifetime annotations.
- **Operator Scope Boundary**: Explicitly defer compound assignments (`+=`, `-=`) and bitwise operators to Phase 2 expression desugaring and standard library functions.
- **Parser Error Diagnostics**: Guarantee informative `ParseError` diagnostics with exact source spans (`Span`) instead of compiler panics on malformed syntax.

## Capabilities

### New Capabilities
<!-- None: language grammar and parsing are owned by the existing `syntax` capability. -->

### Modified Capabilities
- `syntax`: Add formal requirements and scenarios for lexical tokens (string escapes, nested comments, numeric literals), Pratt expression precedence and associativity, trailing lambda and closure blocks, newline-sensitive statement and return expressions, mandatory parentheses on control flow, struct borrow prohibitions, and contextual keywords.

## Impact

- **`crates/prism-lexer`**: Tokenizer implementation, token kinds, string escape validation, span tracking, and lexical test fixtures.
- **`crates/prism-parser`**: Pratt parser implementation, operator precedence table, closure block parsing, struct field validation, and negative diagnostic tests.
- **Downstream Crates**: Serves as the verified foundation for `prism-ast`, `prism-abi`, and the upcoming Phase 2 type checker.
