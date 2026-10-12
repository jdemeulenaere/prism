# Design

## Context

The Prism bootstrap compiler is structured as a modular Rust workspace. `prism-lexer` and `prism-parser` form the initial front-end pipeline, translating raw source text into the syntax trees defined in `prism-ast`, which are subsequently consumed by `prism-abi` (declaration hashing) and `prism-resolver` (package graph resolution).

Language invariants require:
- Default `private` visibility with explicit `package` and `public` export modifiers.
- Kotlin-style ergonomics (semicolon-optional, trailing lambdas, `when` expressions, `compose fun` modifier).
- Value semantics, explicit stack borrows (`&T`, `&mut T`), and `Heap<T>` recursive types.
- Strict prohibition of borrowed references in struct definitions (Invariant 4).
- Unambiguous Pratt operator precedence and robust error reporting with exact source spans (`Span`).

## Goals / Non-Goals

**Goals:**
- Provide a fast, streaming hand-written lexer in `prism-lexer` with zero compiler crashes, tracking byte/line/column spans and newline boundaries (`is_preceded_by_newline`).
- Support nested block comments (`/* ... /* ... */ ... */`), complete string/char escape sets including unicode code points (`\u{...}`), rejection of raw newlines in single-line strings, and all numeric formats (hex, binary, scientific notation, digit separators).
- Provide a Pratt parser in `prism-parser` for expressions with full precedence levels (from assignment up to call and postfix) and non-associative assignment.
- Parse trailing lambda blocks (`Column { ... }`, `Button(id) { ... }`) and multi-statement closures (`{ item -> val x = item; println(x) }`).
- Mandate parentheses on `if (...)` and `when (...)` to prevent LL(1) Pratt grammar conflicts with trailing lambdas.
- Disambiguate newline-sensitive statements, returns (`return` vs `return expr`), and leading parenthesis calls.
- Treat `compose` as a contextual keyword usable as an identifier in member access and parameter positions.
- Enforce Invariant 4 at parse time by rejecting `&T` and `&mut T` in struct field declarations.
- Implement comprehensive unit and integration test coverage across all grammatical features, including negative error diagnostic tests.

**Non-Goals:**
- Type inference, type checking, or borrow checking (Phase 2).
- Resolving imports or scanning filesystem packages (handled by `prism-resolver`).
- Extracting ABI interface hashes or `.abi` cache files (handled by `prism-abi`).
- Compound assignments (`+=`, `-=`) and bitwise operators (`&`, `|`, `^`), which are deferred to Phase 2 expression desugaring and standard library functions.
- Multi-token diagnostic recovery (resynchronization after multiple errors); Phase 1 implements clean fail-fast error reporting returning structured `ParseError` without panics.
- IR lowering and Cranelift JIT compilation (Phase 3).

## Decisions

### 1. Hand-Written Streaming Lexer vs Parser Generator
- **Decision**: Implement a hand-written streaming tokenizer in `crates/prism-lexer`.
- **Rationale**: A hand-written lexer over UTF-8 byte slices provides zero external build dependencies, optimal tokenization throughput, and precise control over newline tracking (`is_preceded_by_newline`), nested comments, and contextual token disambiguation (such as `4..10` vs `4.10`).
- **Alternatives Considered**:
  - `logos`: Rejected to keep the bootstrap compiler free of procedural macro code generators and allow custom newline tracking on tokens.

### 2. Pratt Parsing for Expressions & Operator Precedence Hierarchy
- **Decision**: Use Top-Down Operator Precedence (Pratt) parsing for expressions and recursive-descent for statements and declarations. Assignment (`=`) is non-associative, disallowing chained assignments (`a = b = c`). The Elvis operator (`?:`) is positioned strictly above comparison (`<`, `<=`, `>`, `>=`) and equality (`==`, `!=`), but below range (`..`) and arithmetic sum (`+`, `-`).
- **Rationale**: Pratt parsing cleanly defines operator precedence in a single compact lookup table (`Precedence`). Placing Elvis above comparison guarantees expressions like `age ?: 0 >= 18` parse naturally as `(age ?: 0) >= 18`, while placing it below sum guarantees `base ?: offset + 10` parses as `base ?: (offset + 10)`. Rejecting chained assignments eliminates error-prone assignment expressions and aligns with modern languages.
- **Alternatives Considered**:
  - Right-associative chained assignments: Rejected to prevent accidental assignment-as-expression bugs.
  - LALR / LR parser generators (`lalrpop`): Rejected because LALR grammars struggle with trailing lambdas, contextual modifiers (`compose fun`), and semicolon-less newline handling.

### 3. Trailing Lambda and Closure Block Grammar
- **Decision**: In `parse_expr_postfix`, if a function call expression or identifier is followed immediately by `{` (and not preceded by a newline), parse `{ ... }` as a trailing lambda argument. In `parse_closure`, after parsing optional closure parameters and `->`, parse a sequence of expressions/statements until `}`, wrapping them into a `BlockExpr`.
- **Rationale**: Closures frequently contain multiple variable declarations and statements (`items.forEach { item -> val s = item * item; println(s) }`). Supporting multi-statement closure bodies is essential for real-world usage.
- **Alternatives Considered**:
  - Restricting closures to single expressions: Rejected because closures with setup logic or loops would be impossible to write.

### 4. Mandatory Parentheses on Control Flow Conditions
- **Decision**: Strictly require parentheses around conditions in `if (...)` and subjects in `when (...)`.
- **Rationale**: If parentheses were optional (`if isValid { doWork() }`), Pratt lookahead would bind `{ doWork() }` as a trailing lambda call on `isValid` (`isValid({ doWork() })`). Mandating parentheses eliminates this ambiguity cleanly, adhering to Kotlin conventions.
- **Alternatives Considered**:
  - Complex multi-token lookahead to determine whether `{` begins a then-block or trailing lambda: Fragile and prone to parsing bugs.

### 5. Newline-Sensitive Call and Statement Disambiguation
- **Decision**: In Pratt `current_precedence()`, suppress `Precedence::Call` for `LParen` if `is_preceded_by_newline` is true. Likewise, if `return` is followed by a newline, `RBrace`, or `Eof`, treat it as an empty return producing unit `()`.
- **Rationale**: Prevents accidental function invocation across lines (e.g. `cleanup()\n(1 + 2).toString()`) and cleanly resolves ASI statement boundaries without mandatory semicolons.
- **Alternatives Considered**:
  - Mandatory semicolons: Rejected to maintain Prism's modern developer ergonomics.

### 6. Contextual Keyword Handling for `compose`
- **Decision**: Treat `compose` as a soft keyword that is only interpreted as a declaration modifier when immediately preceding `fun`. In expression and identifier positions (such as member access `pipeline.compose()`), treat it as an identifier.
- **Rationale**: Prevents reserving common words that collide with standard API naming conventions.

### 7. Invariant 4 Enforcement in Struct Fields
- **Decision**: In `parse_struct`, check each parsed field type. If the type is `TypeKind::Reference` (`&T` or `&mut T`), emit a `ParseError` rejecting the declaration.
- **Rationale**: Enforces Invariant 4 directly at the AST generation boundary, ensuring no struct can ever introduce lifetime dependencies.

## Risks / Trade-offs

- **[Range vs Floating-Point Ambiguity]** `4..10` could naively be lexed as float `4.` followed by `.10`.
  - *Mitigation*: In `lex_number`, check lookahead when encountering `.`: if the character following `.` is not an ASCII digit, stop the number at `4` and let the next token be `DotDot`.
- **[Package Declaration vs Package Visibility Keyword]** `package foo.bar` is a package declaration; `package fun helper()` is a visibility modifier.
  - *Mitigation*: Lookahead helper `is_package_decl_ahead()` inspects whether the token after `package` is followed by identifiers and dots rather than declaration keywords (`fun`, `struct`, `enum`, `val`, `var`).
- **[Fail-Fast Error Diagnostics vs Full Recovery]** In Phase 1, the parser bails out on the first syntax error, returning structured `ParseError` with exact spans.
  - *Mitigation*: While multi-error synchronization tokens (e.g. skipping to next top-level keyword) will be added in Phase 2 for LSP support, Phase 1 prioritizes zero panics, deterministic errors, and exact source spans.
