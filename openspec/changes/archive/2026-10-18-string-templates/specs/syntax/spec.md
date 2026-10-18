# Syntax Specification Delta

## MODIFIED Requirements

### Requirement: Lexical Tokenization and Escape Sequences
The compiler MUST tokenize source text into a stream of typed tokens with accurate source spans, processing whitespace, line comments, nested block comments, and escape sequences while rejecting raw unescaped newlines in single-line strings, outside the expression parts of string templates (see "String Templates"). A single byte order mark (U+FEFF) at the very start of a source file MUST be ignored; a U+FEFF anywhere else is an unexpected character.

#### Scenario: Columns count characters
- **WHEN** non-ASCII text precedes a token on its line (`/* café */ fun`)
- **THEN** the token's column counts characters, not UTF-8 bytes (`fun` is at column 12)

#### Scenario: Leading byte order mark ignored
- **WHEN** a source file begins with a byte order mark (`\u{FEFF}package app\nfun main() {}`)
- **THEN** the mark yields no token and takes no column, so `package` is at line 1, column 1 and every later token has the same line and column as without the mark

#### Scenario: Byte order mark elsewhere rejected
- **WHEN** U+FEFF appears anywhere but the first character of the file, including a second mark after the first
- **THEN** the lexer emits an `Error` token for it, as for any other unexpected character

#### Scenario: Declaration and argument spans
- **WHEN** a function has no body (`fun a(): Int` followed by blank lines), a call has a named argument (`n = 1`) or a trailing lambda, or a closure body holds a single `val`
- **THEN** the function's span ends at its signature, a named argument's span covers `n = 1`, a trailing lambda argument's span covers only the lambda, and a closure body block spans from `->` to `}` so it never shares a span with its statement

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

#### Scenario: Unknown escape sequence rejected
- **WHEN** a string or char literal contains a backslash followed by a character other than `n`, `r`, `t`, `\`, `"`, `'`, `0`, `$` or `u` (`"\q"`, `'\é'`)
- **THEN** tokenizer emits an `Error` token naming the unknown escape, without panicking on multi-byte characters

#### Scenario: Dollar sign escape
- **WHEN** a string or char literal contains `\$` (`"\$count costs \$5"`, `'\$'`)
- **THEN** the string token holds the text `$count costs $5` with no template part, and the char token holds `$`

#### Scenario: Empty char literal rejected
- **WHEN** a char literal has no character (`''`)
- **THEN** tokenizer emits an `Error` token

#### Scenario: Unterminated string literal error
- **WHEN** a string literal is unclosed before end-of-file
- **THEN** tokenizer emits an `Error` token with the unterminated literal span without panicking

#### Scenario: Joined operator split where the grammar needs its parts
- **WHEN** a generic argument list is closed directly before `=` (`val y: Heap<Int>= x`), or `&&` appears where a prefix borrow is expected (`&&x`, `&&Int`)
- **THEN** the parser reads `>=` as `>` followed by `=`, and `&&` as two borrows, while `a && b` stays a logical and

## ADDED Requirements

### Requirement: String Templates
A string literal MUST be a template when it holds a `$` followed by an ASCII letter or `_`, which begins a name part, or by `{`, which begins an expression part. A name part is the longest run of identifier characters after the `$`, and MUST be an identifier or `this`; another keyword after `$` MUST be rejected (`E2001`). Any other `$` MUST be the character `$`. A literal without parts MUST remain a plain string literal.

#### Scenario: Adjacent parts and this
- **WHEN** tokenizing `"$a$b"` and `"[$this]"`
- **THEN** the first holds the name parts `a` and `b` with no text between them, and the second the text `[`, the name part `this` and the text `]`

#### Scenario: A dollar sign that starts no part
- **WHEN** tokenizing `"$5, $ x, a$, $$y, $é"`
- **THEN** the result is a template whose text is `$5, $ x, a$, $` followed by the name part `y` and the text `, $é`; and tokenizing `"cost: $5"` gives a single plain string token holding `cost: $5`

#### Scenario: A keyword after a dollar sign rejected
- **WHEN** a file contains `val s = "$if"` or `val t = "$null"`
- **THEN** the lexer emits an `Error` token naming the keyword, and `prism check` reports `E2001`

### Requirement: Expression Parts Match Their Braces
An expression part MUST end at the `}` that matches its `${`: braces of the expression itself, such as those of closures, blocks and `when`, MUST be matched first, and braces inside nested string and char literals and comments MUST NOT count. An expression part MAY hold nested literals and templates, comments and newlines. The text between parts follows the rules of string literals: escapes are decoded and a raw newline is rejected.

#### Scenario: Braces and strings inside an expression part
- **WHEN** tokenizing `"${when (n) { 1 -> "}" else -> "$n {" }} done"`
- **THEN** the template has one expression part, holding the whole `when` with its nested literal `"}"` and nested template `"$n {"`, followed by the text ` done`

#### Scenario: Comment-collecting lexer inside templates
- **WHEN** tokenizing `"x ${a /* why */ + b} // not a comment"` in comment-collecting mode
- **THEN** the tokens are those of normal lexing, the block comment `/* why */` is collected, and the text ` // not a comment` is a text run, not a comment

### Requirement: Template Tokens
The lexer MUST emit a template as a flat run of tokens with their source spans: a template start (the opening quote), its text runs (never empty) and parts in source order, and a template end (the closing quote). A name part MUST be one token holding the name, and an expression part an opening token (`${`), the ordinary tokens of the expression and a closing token (its `}`). An unterminated template or expression part MUST yield an `Error` token without panicking.

#### Scenario: Template tokens
- **WHEN** tokenizing `"a $b ${c + d}!"`
- **THEN** the tokens are a template start, the text `a `, the name part `b`, the text ` `, an expression start, `c`, `+`, `d`, an expression end, the text `!` and a template end, followed by the end of the file

#### Scenario: Unterminated templates rejected
- **WHEN** a template's text reaches a newline (`val s = "a $b` followed by a newline), an expression part is still open at the end of the file (`val s = "${a` followed by a newline and the end of the file), or a nested template inside an expression part is unterminated
- **THEN** the lexer emits an `Error` token without panicking, and `prism check` reports `E2001`

### Requirement: Template Expressions
The parser MUST turn a template into a template expression holding its text runs and parts in source order. A name part MUST be the identifier (or `this`) it names, with the span of the name, so `"$x"` and `"${x}"` give the same tree. An expression part MUST hold exactly one expression, parsed with newlines insignificant as inside parentheses; an empty part or a statement MUST be rejected (`E2001`). A template is an atom that postfix operators apply to.

#### Scenario: Template expression tree
- **WHEN** parsing `val s = "Hi $name, ${a + b}!".length`
- **THEN** the initializer reads the member `length` of a template whose parts are the text `Hi `, the identifier `name`, the text `, `, the binary expression `a + b` and the text `!`

#### Scenario: Name and braced parts are the same expression
- **WHEN** parsing `val a = "$x"` and `val b = "${x}"`
- **THEN** both initializers are a template holding only the identifier `x`

#### Scenario: Newlines inside an expression part
- **WHEN** parsing `val s = "${a +\n    b}"`
- **THEN** the template's part is the binary expression `a + b`

#### Scenario: An empty or statement part rejected
- **WHEN** a file contains `val s = "${}"` or `val t = "${val x = 1}"`
- **THEN** `prism check` reports `E2001`
