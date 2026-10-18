# Syntax Specification Delta

## ADDED Requirements

### Requirement: Comments Are Available to Tools
The lexer MUST offer a comment-collecting mode for tools such as the formatter. In that mode it MUST yield exactly the token stream of normal lexing, with the same kinds, spans and newline flags, and also every comment in source order, each with its kind (a `//` line comment, which ends before the newline, or a `/* */` block comment, nested comments included) and its span. Normal lexing, which compilation uses, MUST NOT collect comments.

#### Scenario: Comment-collecting lexer yields the same tokens
- **WHEN** tokenizing `a /* b /* c */ d */ + // e\nf` in comment-collecting mode
- **THEN** the tokens are those of normal lexing (`a`, `+`, `f` preceded by a newline, end of file), and the comments are the block comment `/* b /* c */ d */` and the line comment `// e`, in that order, with their spans

#### Scenario: Comment markers inside literals are not comments
- **WHEN** tokenizing `val s = "// not a comment /* either */"` in comment-collecting mode
- **THEN** no comment is collected and the string literal keeps its text
