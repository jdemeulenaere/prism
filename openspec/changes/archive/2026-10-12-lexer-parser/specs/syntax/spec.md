# Spec Delta

## ADDED Requirements

### Requirement: Lexical Tokenization and Escape Sequences
The compiler MUST tokenize source text into a stream of typed tokens with accurate source spans, processing whitespace, line comments, nested block comments, and escape sequences while rejecting raw unescaped newlines in single-line strings.

#### Scenario: Nested block comments ignored
- **WHEN** source code contains nested block comments (`/* outer /* inner */ still comment */`)
- **THEN** tokenizer ignores the comments and yields adjacent tokens without error

#### Scenario: String escape and unicode sequence
- **WHEN** a string literal contains escape characters and unicode sequences (`"Hello\n\u{1F680}"`)
- **THEN** the token kind is `String` containing the decoded characters and emoji

#### Scenario: Raw newline in single-line string rejected
- **WHEN** a single-line string literal contains an unescaped newline before its closing quote
- **THEN** tokenizer emits an `Error` token with the span of the unterminated string without consuming subsequent lines

#### Scenario: Malformed unicode escape sequence error
- **WHEN** a string literal contains a malformed unicode escape (`"\u12"`)
- **THEN** tokenizer reports an `Error` token without consuming the closing quote

#### Scenario: Unterminated string literal error
- **WHEN** a string literal is unclosed before end-of-file
- **THEN** tokenizer emits an `Error` token with the unterminated literal span without panicking

### Requirement: Numeric Literal Formats
The lexer MUST parse decimal integers, hexadecimal, binary, floating-point numbers with fractional decimals, visual digit separators (`_`), and scientific notation.

#### Scenario: Radix integers with digit separators
- **WHEN** numeric literals are written with prefixes and separators (`0xFF_FF`, `0b1010_0001`)
- **THEN** the tokenizer emits `Int(65535)` and `Int(161)`

#### Scenario: Floating point with scientific notation
- **WHEN** a float literal is written with exponents (`3.14e-2`)
- **THEN** the tokenizer emits `Float(0.0314)`

#### Scenario: Disambiguation between float and range operator
- **WHEN** integer literal is immediately followed by range operator (`4..10`)
- **THEN** tokenizer emits `Int(4)`, `DotDot`, and `Int(10)` rather than a malformed float

### Requirement: Pratt Expression Precedence and Associativity
The parser MUST parse expressions according to Pratt operator precedence, correctly ordering assignment, Elvis, logical, comparison, range, arithmetic, prefix, postfix, and function call operators.

#### Scenario: Arithmetic and comparison precedence
- **WHEN** parsing expression `a + b * c > d`
- **THEN** operator precedence structures AST as `((a + (b * c)) > d)`

#### Scenario: Non-associative assignment
- **WHEN** code attempts chained assignment (`a = b = c`)
- **THEN** the parser rejects the expression with a compile error requiring separate statements

#### Scenario: Elvis operator binds tighter than comparison
- **WHEN** parsing `val eligible = age ?: 0 >= 18`
- **THEN** the Elvis operator binds tighter than comparison: `((age ?: 0) >= 18)`

#### Scenario: Elvis operator binds looser than arithmetic
- **WHEN** parsing `val total = base ?: offset + 10`
- **THEN** arithmetic addition binds tighter than the Elvis fallback: `(base ?: (offset + 10))`

#### Scenario: Newline suppresses call precedence on leading parenthesis
- **WHEN** an expression starting with `(` is on a new line following a call (`cleanup()\n(1 + 2).toString()`)
- **THEN** the parser treats `(1 + 2)` as a separate statement rather than a call argument to `cleanup()`

### Requirement: Trailing Lambda and Multi-Statement Closure Syntax
The parser MUST support trailing lambda syntax for function invocations, and closure blocks MUST support optional parameter lists followed by multi-statement bodies.

#### Scenario: Invocation with argument list and trailing lambda
- **WHEN** parsing `Button(onClick = handler) { Text("Click") }`
- **THEN** the trailing block `{ Text("Click") }` is parsed as the final lambda argument to `Button`

#### Scenario: Invocation with trailing lambda only
- **WHEN** parsing `Column { Item() }`
- **THEN** the call parses without parentheses and passes the block as the lambda argument

#### Scenario: Closure with parameters and multi-statement body
- **WHEN** parsing a closure with parameters and multiple statements (`items.forEach { item -> val s = item * item; println(s) }`)
- **THEN** the closure body is parsed as a block containing both statements

### Requirement: Newline-Sensitive Return and Statement Termination
The parser MUST recognize newlines as expression and statement delimiters, parsing `return` followed by a newline as returning unit `()`, while `return <expr>` on the same line parses the expression.

#### Scenario: Bare return followed by newline
- **WHEN** a function block contains `return` followed by a newline and subsequent expression `42`
- **THEN** `return` parses as returning unit `()`, and `42` parses as a separate statement

#### Scenario: Return with expression on same line
- **WHEN** code contains `return 42\n`
- **THEN** `42` parses as the returned value expression of `return`

#### Scenario: Semicolon-free statement termination
- **WHEN** statements appear on consecutive lines without semicolons (`val a = 1\nval b = 2`)
- **THEN** each statement is parsed independently into the enclosing block

### Requirement: Mandatory Parentheses on Control Flow Conditions
The grammar MUST mandate parentheses around `if` conditions and `when` subjects to prevent grammatical ambiguity with trailing lambda blocks.

#### Scenario: If condition with required parentheses
- **WHEN** parsing `if (isValid) { doWork() }`
- **THEN** `isValid` is parsed as the condition and `{ doWork() }` is parsed as the then-branch block

#### Scenario: When subject with required parentheses
- **WHEN** parsing `when (status) { 0 -> "ok", else -> "fail" }`
- **THEN** `status` is parsed as the subject and the subsequent block is parsed as pattern branches

### Requirement: Struct Field Borrow Prohibition
Struct declarations MUST NOT declare fields with borrowed reference types (`&T` or `&mut T`), enforcing Invariant 4 at parse time.

#### Scenario: Struct field declaring borrowed reference rejected
- **WHEN** a struct definition declares a field with reference type (`struct View { val data: &String }`)
- **THEN** the parser emits a compile error rejecting borrowed references in structs

### Requirement: Contextual Keyword Identifier Permissibility
Modifiers such as `compose` MUST function contextually, permitting their use as regular identifiers in member access and parameter declarations.

#### Scenario: Member access on compose identifier
- **WHEN** code accesses a property or method named compose (`pipeline.compose(step)`)
- **THEN** `compose` is parsed as a valid identifier expression rather than a modifier keyword
