# Syntax Delta

## ADDED Requirements

### Requirement: Library Declaration Modifiers
A top-level `fun` MAY carry `external` or `intrinsic`, and an extension property `intrinsic`. The modifier order MUST be visibility, then `external` or `intrinsic`, then `compose`, then the receiver mode. Such a declaration MUST have no body (`=` or a block), and one with both modifiers, or either twice, MUST be rejected with `E2001`. Which files may use them is the rule of `standard-library`.

#### Scenario: Library declarations parse
- **WHEN** parsing `private external fun prism_rt_exit(status: Int): Nothing`, `public intrinsic fun <T> print(value: &T)`, `public intrinsic mut fun <T> List<T>.add(element: T)` and `public intrinsic compose fun <T : Copy> remember(calculation: () -> T): T`
- **THEN** each parses as a body-less function declaration with its modifier, visibility and receiver mode

#### Scenario: A library declaration with a body rejected
- **WHEN** parsing `external fun f(): Int = 1` or `intrinsic fun g() { }`
- **THEN** the parser reports `E2001`, saying that an external or intrinsic declaration has no body

#### Scenario: Library modifiers in the wrong order rejected
- **WHEN** parsing `compose intrinsic fun A() { }`, `intrinsic public fun b(): Int`, `external intrinsic fun c(): Int` or `mut external fun Int.d()`
- **THEN** the parser reports `E2001`

### Requirement: Function Bodies Are Required
A `fun` declaration without `external` or `intrinsic` MUST have a body, an `=` expression or a block. Without one the parser MUST report `E2001`, saying that a function needs a body.

#### Scenario: A function without a body rejected
- **WHEN** parsing `fun a(): Int` followed by a newline and `fun b() {}`
- **THEN** the parser reports `E2001` at the end of `a`'s signature

### Requirement: Intrinsic Extension Properties
`intrinsic val`, optionally after a visibility modifier, MUST declare an extension property: an optional type parameter list, a receiver type, `.`, the property name, `:` and its type (`public intrinsic val <T> List<T>.size: Int`), with no initializer. A `val` declaration with a receiver and without `intrinsic`, and an intrinsic property with an initializer, MUST be rejected with `E2001`.

#### Scenario: Intrinsic properties parse
- **WHEN** parsing `public intrinsic val String.length: Int` and `public intrinsic val <T> List<T>.indices: Range`
- **THEN** each parses as an extension property with its receiver type, name and type

#### Scenario: Extension properties outside the library form rejected
- **WHEN** parsing `val String.twice: Int = 2` or `intrinsic val String.a: Int = 1`
- **THEN** the parser reports `E2001`

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
- **WHEN** an intrinsic function has no body (`intrinsic fun a(): Int` followed by blank lines), a call has a named argument (`n = 1`) or a trailing lambda, or a closure body holds a single `val`
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

### Requirement: Contextual Keyword Identifier Permissibility
Modifiers such as `compose`, `consuming`, `escaping`, `external` and `intrinsic` MUST function contextually, permitting their use as regular identifiers in member access and parameter declarations. `consuming` is a modifier only directly before `fun` at the start of a declaration. `external` and `intrinsic` are modifiers only at the start of a top-level declaration, after any visibility modifier and directly before `compose`, `mut`, `consuming`, `fun` or `val`. `escaping` is a modifier only at the start of a parameter's type and directly before `(` or `compose`; elsewhere it is a name, also a type name.

#### Scenario: Member access on compose identifier
- **WHEN** code accesses a property or method named compose (`pipeline.compose(step)`)
- **THEN** `compose` is parsed as a valid identifier expression rather than a modifier keyword

#### Scenario: Named argument called compose
- **WHEN** a call passes a named argument called `compose` (`g(compose = 1)`) or a closure declares a parameter called `compose` (`{ compose -> compose }`)
- **THEN** `compose` is parsed as the argument name or parameter name, not as a modifier or an assignment

#### Scenario: consuming as an identifier
- **WHEN** parsing `fun consuming(consuming: Int): Int = consuming`, `fun f() { val consuming = 1; g(consuming = consuming) }` and `x.consuming()`
- **THEN** each `consuming` parses as a function, parameter, variable, argument or member name, not as a modifier

#### Scenario: escaping as an identifier
- **WHEN** parsing `fun escaping(escaping: Int): Int = escaping`, `fun f(x: escaping, y: escaping?) {}`, `fun g() { val escaping = 1; h(escaping = escaping) }` and `x.escaping()`
- **THEN** each `escaping` parses as a function, parameter, variable, argument, member or type name, not as a modifier

#### Scenario: external and intrinsic as identifiers
- **WHEN** parsing `fun external(intrinsic: Int): Int = intrinsic`, `fun f() { val external = 1; g(intrinsic = external) }` and `x.intrinsic()`
- **THEN** each `external` and `intrinsic` parses as a function, parameter, variable, argument or member name, not as a modifier

### Requirement: Receiver Mode Modifiers
`mut` or `consuming` MAY appear directly before `fun` in a declaration that has a receiver, after any visibility, `external`, `intrinsic` and `compose` modifiers, to declare the receiver mode. On a declaration without a receiver, before a visibility, `external`, `intrinsic` or `compose` modifier, or twice, they MUST be rejected with `E2001`.

#### Scenario: Mutating and consuming extensions
- **WHEN** parsing `mut fun User.rename(n: String) { name = n }`, `public consuming fun User.toAccount(): Account = Account(this)` and `package compose mut fun Counter.Increment(): Unit { }`
- **THEN** the declarations parse with the receiver modes mutable, consuming and mutable

#### Scenario: Receiver modifier without a receiver rejected
- **WHEN** parsing `mut fun f() { }` or `consuming fun g(): Int = 1`
- **THEN** the parser reports `E2001`, saying that `mut` and `consuming` modify the receiver of an extension function (`mut fun T.name()`)

#### Scenario: Modifiers in the wrong order rejected
- **WHEN** parsing `mut public fun User.a() { }`, `consuming compose fun User.b() { }` or `mut consuming fun User.c() { }`
- **THEN** the parser reports `E2001`
