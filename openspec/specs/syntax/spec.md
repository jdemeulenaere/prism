# Prism Syntax & Grammar Specification

## Purpose
The syntax specification defines the concrete lexical and grammatical rules of the Prism programming language, ensuring Kotlin-like developer ergonomics combined with unambiguous, fast Pratt parsing.

## Requirements

### Requirement: Default Private Visibility and Export Modifiers
The compiler MUST treat any top-level declaration without an explicit visibility modifier as `private` to its declaring source file.

#### Scenario: Declaration without visibility modifier
- **WHEN** a function, struct, or enum is declared without a visibility modifier (`fun foo(): Int = 42`)
- **THEN** the symbol is accessible only within the declaring source file and is excluded from the exported ABI

#### Scenario: Package visibility modifier
- **WHEN** a declaration is annotated with the `package` modifier (`package fun helper(): Int`)
- **THEN** the symbol is accessible only by files sharing the identical package directory

#### Scenario: Public visibility modifier
- **WHEN** a declaration is annotated with the `public` modifier (`public fun api(): Int`)
- **THEN** the symbol is accessible across all packages via module imports

### Requirement: Mandatory Explicit Types on Exported Declarations
All declarations marked `public` or `package` MUST declare explicit parameter types and explicit return types.

#### Scenario: Exported function with explicit types
- **WHEN** a `public` or `package` function declares all parameter types and a return type (`public fun add(a: Int, b: Int): Int = a + b`)
- **THEN** the declaration is valid and its ABI signature can be extracted in Phase 1 without inspecting the body

#### Scenario: Exported function omitting return type
- **WHEN** a `public` or `package` function omits its return type (`public fun compute(x: Int) = x * 2`)
- **THEN** the compiler MUST emit a compile error requiring an explicit return type

#### Scenario: Private function with inferred return type
- **WHEN** a private function omits its return type (`fun localCompute(x: Int) = x * 2`)
- **THEN** type inference is permitted and the compiler infers the return type during body typechecking

### Requirement: Variable Bindings and Immutability
Variables MUST be immutable by default using `val`, while mutable variables MUST be explicitly declared with `var`. `var` declares a mutable local only: a top-level declaration is always a `val` (see the [`ownership`](../ownership/spec.md) spec, "No Mutable Top-Level State").

#### Scenario: Reassigning immutable val binding
- **WHEN** code attempts to reassign a variable declared with `val` (`val x = 10; x = 20`)
- **THEN** the compiler MUST reject the reassignment with a compile error

#### Scenario: Reassigning mutable var binding
- **WHEN** code reassigns a variable declared with `var` (`var x = 10; x = 20`)
- **THEN** the mutation is valid

### Requirement: Value Structs and Algebraic Enums
The grammar MUST accept value struct declarations with owned fields, and algebraic enum declarations whose variants carry payloads, including recursive payloads wrapped in `Heap<T>`. Inline layout is planned (see `docs/architecture.md`).

#### Scenario: Value struct definition
- **WHEN** a struct is declared with fields (`struct Point { val x: Double, val y: Double }`)
- **THEN** the declaration parses into a struct with by-value fields `x` and `y` of type `Double`

#### Scenario: Recursive enum with Heap indirection
- **WHEN** an enum variant contains recursive instances wrapped in `Heap<T>` (`enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }`)
- **THEN** the declaration parses into an enum whose variants carry `Heap<Expr>` payloads, and the compiler accepts it

### Requirement: Expression-Oriented Control Flow
Control flow constructs (`if` and `when`) MUST function as first-class expressions producing values.

#### Scenario: If expression assignment
- **WHEN** an `if-else` block is assigned to a variable (`val result = if (condition) 10 else 20`)
- **THEN** both branches are typechecked to a common supertype and the value is assigned

#### Scenario: When pattern matching
- **WHEN** a `when` expression matches over values, ranges or constructor patterns (`when (x) { 0 -> "zero", in 1..10 -> "small", else -> "large" }`, `when (s) { Shape.Rect(val w, _) if w > 1.0 -> "wide"; _ -> "other" }`)
- **THEN** each branch parses into its patterns (a value, a constructor pattern, a wildcard `_`, a `val` or `var` binding, an `in` range, several of these separated by commas, or `else`; see the [`patterns`](../patterns/spec.md) spec), an optional guard, and its branch expression, in source order

#### Scenario: When branch without a pattern rejected
- **WHEN** a `when` branch has no pattern before its arrow (`when (x) { -> 1 }`)
- **THEN** the parser reports a syntax error at the arrow

### Requirement: Null Safety and Optional Types
The language MUST enforce compile-time null safety where nullable types are designated with `T?`.

#### Scenario: Assigning null to non-nullable type
- **WHEN** code attempts to assign `null` to a non-nullable variable (`val name: String = null`)
- **THEN** the compiler MUST reject the assignment at compile time

#### Scenario: Safe navigation and Elvis operator
- **WHEN** code accesses a property on a nullable reference using `?.` and provides a fallback with `?:` (`val len = name?.length ?: 0`)
- **THEN** `?.` parses as safe navigation and `?:` as the Elvis fallback

### Requirement: First-Class Compose UI Keyword
The grammar MUST recognize `compose` as a native modifier on function declarations (`compose fun`).

#### Scenario: Composable function declaration
- **WHEN** a function is declared with `compose fun CounterView()`
- **THEN** the compiler parses it as a composable UI function eligible for slot-table transformation

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

### Requirement: Comments Are Available to Tools
The lexer MUST offer a comment-collecting mode for tools such as the formatter. In that mode it MUST yield exactly the token stream of normal lexing, with the same kinds, spans and newline flags, and also every comment in source order, each with its kind (a `//` line comment, which ends before the newline, or a `/* */` block comment, nested comments included) and its span. Normal lexing, which compilation uses, MUST NOT collect comments.

#### Scenario: Comment-collecting lexer yields the same tokens
- **WHEN** tokenizing `a /* b /* c */ d */ + // e\nf` in comment-collecting mode
- **THEN** the tokens are those of normal lexing (`a`, `+`, `f` preceded by a newline, end of file), and the comments are the block comment `/* b /* c */ d */` and the line comment `// e`, in that order, with their spans

#### Scenario: Comment markers inside literals are not comments
- **WHEN** tokenizing `val s = "// not a comment /* either */"` in comment-collecting mode
- **THEN** no comment is collected and the string literal keeps its text

### Requirement: Numeric Literal Formats
The lexer MUST parse decimal integers, hexadecimal, binary, floating-point numbers with fractional decimals, visual digit separators (`_`), and scientific notation.

#### Scenario: Radix integers with digit separators
- **WHEN** numeric literals are written with prefixes and separators (`0xFF_FF`, `0b1010_0001`)
- **THEN** the tokenizer emits `Int(65535)` and `Int(161)`

#### Scenario: Floating point with scientific notation
- **WHEN** a float literal is written with exponents (`3.14e-2`)
- **THEN** the tokenizer emits `Float(0.0314)`

#### Scenario: Out-of-range numeric literals rejected
- **WHEN** a float literal overflows to infinity (`1e999`) or an integer literal exceeds the `Int` range (`9223372036854775809`)
- **THEN** tokenizer emits an `Error` token instead of an infinite or wrapped value

#### Scenario: Minimum Int literal
- **WHEN** unary minus is applied directly to `9223372036854775808` with no postfix operator after it (`-9223372036854775808`)
- **THEN** the parser produces the `Int` literal `-9223372036854775808`, while `9223372036854775808` alone is rejected as out of range

#### Scenario: Disambiguation between float and range operator
- **WHEN** integer literal is immediately followed by range operator (`4..10`)
- **THEN** tokenizer emits `Int(4)`, `DotDot`, and `Int(10)` rather than a malformed float

### Requirement: Pratt Expression Precedence and Associativity
The parser MUST parse expressions according to Pratt operator precedence, correctly ordering assignment, Elvis, logical, comparison, range, arithmetic, prefix, postfix, and function call operators. Comparison operators (`<`, `<=`, `>`, `>=`) MUST be non-associative: an operand of a comparison MUST NOT itself be an unparenthesized comparison.

#### Scenario: Arithmetic and comparison precedence
- **WHEN** parsing expression `a + b * c > d`
- **THEN** operator precedence structures AST as `((a + (b * c)) > d)`

#### Scenario: Non-associative assignment
- **WHEN** code attempts chained assignment (`a = b = c`)
- **THEN** the parser rejects the expression with a compile error requiring separate statements

#### Scenario: Non-associative comparison
- **WHEN** parsing `a < b < c` or `a < b > c`
- **THEN** the parser reports `E2001` at the second operator, suggesting `&&` between two comparisons, while `(a < b) == c` and `a < b == c > d` parse

#### Scenario: Invalid assignment target rejected
- **WHEN** the left side of `=` is not a variable, a `.` field access, an index expression or a write through `*` (`a + b = c`, `1 = 2`, `g() = 1`, `a?.b = c`)
- **THEN** the parser rejects the assignment with a syntax error at the target

#### Scenario: Elvis operator binds tighter than comparison
- **WHEN** parsing `val eligible = age ?: 0 >= 18`
- **THEN** the Elvis operator binds tighter than comparison: `((age ?: 0) >= 18)`

#### Scenario: Elvis operator binds looser than arithmetic
- **WHEN** parsing `val total = base ?: offset + 10`
- **THEN** arithmetic addition binds tighter than the Elvis fallback: `(base ?: (offset + 10))`

#### Scenario: Newline suppresses call precedence on leading parenthesis
- **WHEN** an expression starting with `(` is on a new line following a call (`cleanup()\n(1 + 2).toString()`)
- **THEN** the parser treats `(1 + 2)` as a separate statement rather than a call argument to `cleanup()`

### Requirement: Bounded Nesting Depth
The parser MUST reject source whose expressions or types nest deeper than 256 levels with error `E2002`, and MUST NOT overflow the stack or abort on any input. The limit bounds the height of the syntax tree, so an operator chain counts one level per operator even though it is parsed iteratively.

#### Scenario: Deeply nested expression rejected
- **WHEN** a function body nests 100,000 parentheses, blocks, unary operators or `else if` branches
- **THEN** the parser reports `E2002` at the first token past the limit instead of overflowing the stack

#### Scenario: Long operator chain rejected
- **WHEN** a function body is a chain of 100,000 additions (`a + a + ... + a`)
- **THEN** the parser reports `E2002` rather than building a tree too deep to drop or type-check

#### Scenario: Nesting within the limit accepted
- **WHEN** an expression nests 246 parentheses
- **THEN** it parses without error

### Requirement: Trailing Lambda and Multi-Statement Closure Syntax
The parser MUST support trailing lambda syntax for function invocations, and closure blocks MUST support optional parameter lists followed by multi-statement bodies.

#### Scenario: Invocation with argument list and trailing lambda
- **WHEN** parsing `Button(onClick = handler) { Text("Click") }`
- **THEN** the trailing braces `{ Text("Click") }` are parsed as a closure with zero parameters, passed as the final argument to `Button`

#### Scenario: Invocation with trailing lambda only
- **WHEN** parsing `Column { Item() }`
- **THEN** the call parses without parentheses and passes a zero-parameter closure as its only argument

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

#### Scenario: Bare return before a token that cannot start an expression
- **WHEN** `return` is directly followed by a token that cannot start an expression (`if (c) return else g()`, `when (x) { 0 -> return, else -> 1 }`, `g(return)`)
- **THEN** `return` parses as returning unit `()` and the following token continues the enclosing construct

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
Struct fields and enum variant payloads MUST NOT store a borrowed reference (`&T` or `&mut T`), including one nested in `?` or a type argument, enforcing the zero-cost memory-safety invariant at parse time. A borrow inside a function type MUST be accepted, because it exists only for the duration of a call.

#### Scenario: Struct field declaring borrowed reference rejected
- **WHEN** a struct definition declares a field with reference type (`struct View { val data: &String }`)
- **THEN** the parser emits a compile error rejecting borrowed references in structs

#### Scenario: Enum payload declaring borrowed reference rejected
- **WHEN** an enum variant has a payload of reference type (`enum Slot { Some(&String), Empty }`)
- **THEN** the parser reports `E2001`, pointing at the borrow

#### Scenario: Borrow nested in a field type rejected
- **WHEN** a struct field or enum payload has type `(&String)?` or `Heap<&Int>` (`struct A { val x: (&String)? }`, `struct B(val x: Heap<&Int>)`, `enum E { X((&Int)?) }`)
- **THEN** the parser reports `E2001`, pointing at the nested borrow

#### Scenario: Function-typed field with borrowed parameters accepted
- **WHEN** a struct declares `val onClick: (&User) -> Unit` or `val pick: (&Int) -> &Int`
- **THEN** the declaration parses

### Requirement: Contextual Keyword Identifier Permissibility
`attached`, `capture`, `compose`, `consuming`, `context`, `escaping`, `external` and `intrinsic` MUST be contextual modifiers, usable as identifiers in member access and parameter declarations. `capture` and `consuming` are ones only directly before `fun`; `context`, `external` and `intrinsic` only at a top-level declaration's start, `context` directly before `(`; `attached` and `escaping` only at a parameter type's start. Elsewhere each is a name, `attached` and `escaping` also type names.

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

#### Scenario: context as an identifier
- **WHEN** parsing `fun context(context: Int): Int = context`, `fun f() { val context = 1; g(context = context); context(context) }`, `struct context { val x: Int }` and `x.context()`
- **THEN** each `context` parses as a function, parameter, variable, argument, callee, type or member name, not as a modifier

#### Scenario: attached and capture as identifiers
- **WHEN** parsing `fun attached(attached: Int): Int = attached`, `fun f(x: attached, capture: Int) {}`, `fun g() { val capture = 1; h(attached = capture) }` and `x.capture()`
- **THEN** each `attached` and `capture` parses as a function, parameter, variable, argument, member or type name, not as a modifier

### Requirement: Braces Are Closures in Expression Position
A `{` in expression position MUST start a closure, with or without a parameter header. A `{` MUST start a block only in block position: a function body, an `if`/`else` branch body, a `when` branch body, or a statement in a block or closure body. A closure without a `->` header MUST declare no parameters, and the syntax tree MUST record whether a closure has a header, so that the empty header `{ -> body }` stays distinct from no header. Every closure body MUST be a block.

#### Scenario: Trailing lambda without parameters is a closure
- **WHEN** parsing `Column { Item() }` and `Button(onClick = handler) { Text("Click") }`
- **THEN** each trailing argument is a closure with zero parameters, and its body block contains the call

#### Scenario: Brace argument and initializer are closures
- **WHEN** parsing `g({ 1 })`, `Button(onClick = { count = 1 })` and `val f = { 1 }`
- **THEN** each brace expression is a closure with zero parameters, not an eagerly evaluated block

#### Scenario: Single-expression closure body is a block
- **WHEN** parsing `run { x -> x + 1 }`
- **THEN** the closure has the parameter `x`, and its body is a block whose single statement is `x + 1`

#### Scenario: Braces in block position stay blocks
- **WHEN** parsing `if (c) { a() } else { b() }`, the `when` branch `0 -> { a() }`, and a statement-position `{ val t = 1 }` inside a function body
- **THEN** each brace group is a block, not a closure

#### Scenario: Expression-bodied function returning a closure
- **WHEN** parsing `fun f() = { 1 }`
- **THEN** the function body is a closure with zero parameters, as in Kotlin

#### Scenario: An empty parameter header is recorded
- **WHEN** parsing `run { -> 1 }`, `run { 1 }` and `run { -> }`
- **THEN** the first and third closures have a header declaring zero parameters, and the second has no header

### Requirement: Function Type Syntax
The type grammar MUST accept function types `(A, B) -> R`, including the zero-parameter form `() -> R`, optionally prefixed by `compose` to mark a composable function type. A function type MAY name a receiver before its parameter list, `T.(A, B) -> R`, and `mut` before the receiver marks it mutable, `mut T.(A) -> R`; `compose` comes before `mut`. `()` alone is the unit type, a single parenthesized type without an arrow is that type, and `->` is right-associative.

#### Scenario: Parameterless function type
- **WHEN** parsing `fun f(onClick: () -> Unit) { }`
- **THEN** the parameter type is a non-composable function type with no parameters returning `Unit`

#### Scenario: Composable function type
- **WHEN** parsing `fun Column(content: compose () -> Unit) { }` and `fun Row(item: compose (Int) -> Unit) { }`
- **THEN** each parameter type is a composable function type, and the exported ABI type string starts with `compose `

#### Scenario: Higher-order function type
- **WHEN** parsing `fun f(g: (Int) -> (Int) -> Int) { }`
- **THEN** the parameter type is a function from `Int` to a function from `Int` to `Int`

#### Scenario: Nullable function type needs parentheses
- **WHEN** parsing `fun f(a: (() -> Unit)?, b: () -> Unit?) { }`
- **THEN** `a` is a nullable function type, and `b` is a function type returning `Unit?`

#### Scenario: Function types with a receiver
- **WHEN** parsing `fun Column(content: compose ColumnScope.() -> Unit) { }`, `fun build(f: mut Builder.(String) -> Unit) { }`, `fun later(f: escaping compose mut Builder.() -> Unit) { }` and `fun g(h: Scope.() -> Scope.() -> Int) { }`
- **THEN** `content` is a composable function type with the shared receiver `ColumnScope` and no other parameters, `f` of `build` a function type with the mutable receiver `Builder` and one `String` parameter, `f` of `later` an escaping composable function type with the mutable receiver `Builder`, and `h` a function type with the receiver `Scope` returning a function type with the receiver `Scope`

### Requirement: Declarations in Statement Position Only
A local `val` or `var` declaration MUST appear only as a statement directly inside a block or closure body, and the parser MUST reject it anywhere else with `E2001`. That includes call arguments, operands, assigned values, conditions, returned values, expression function bodies, and unbraced `if`/`else`, `when` or loop bodies. A declaration's initializer MUST NOT be an assignment.

#### Scenario: Declaration as a call argument rejected
- **WHEN** parsing `fun f() { g(val z = 1) }`
- **THEN** the parser reports `E2001`, stating that a declaration is only allowed as a statement

#### Scenario: Declaration as an assigned value rejected
- **WHEN** parsing `fun f() { x = val y = 3 }`
- **THEN** the parser reports `E2001`

#### Scenario: Declaration as an unbraced branch body rejected
- **WHEN** parsing `fun f(c: Bool): Int {\nif (c) val x = 1\nreturn x\n}`
- **THEN** the parser reports `E2001` at `val`, so `x` cannot leak out of the conditional

#### Scenario: Declaration as an unbraced loop body rejected
- **WHEN** parsing `fun f(c: Bool) {\nwhile (c) val x = 1\n}`
- **THEN** the parser reports `E2001` at `val`

#### Scenario: Declaration in a braced branch accepted
- **WHEN** parsing `if (c) { val x = 1 }`
- **THEN** the declaration parses as a statement of the branch block

#### Scenario: Chained assignment through a declaration rejected
- **WHEN** parsing `fun f() { val a = b = c }` or the top-level `val a = b = c`
- **THEN** the parser reports `E2001` rejecting the chained assignment, as it does for `a = b = c`

### Requirement: Newline Before a Binary Operator Ends the Expression
Outside parentheses and brackets, a newline before a binary operator MUST end the current expression, unless the operator is `.`, `?.`, `&&`, `||` or `?:`. Inside `( )` and `[ ]`, newlines MUST NOT end an expression. A `{` opened inside them restores newline sensitivity until its matching `}`.

#### Scenario: Leading minus starts a new statement
- **WHEN** parsing `fun f() {\nval a = b\n-1\n}` and `fun f() {\nif (c) { foo() }\n-1\n}`
- **THEN** each body has two statements, and the second is `-1`

#### Scenario: Leading minus starts a new when branch
- **WHEN** parsing `when (x) {\n0 -> a\n-1 -> b\nelse -> c\n}`
- **THEN** the `when` has three branches with patterns `0`, `-1` and `else`

#### Scenario: Continuation operators join lines
- **WHEN** parsing `val ok = a\n&& b` and `val n = user\n?.name\n?: "anon"`
- **THEN** the operators continue the expression from the previous line

#### Scenario: Newlines inside parentheses and brackets are ignored
- **WHEN** parsing `g(a\n- 1)` and `xs[i\n+ 1]`
- **THEN** the argument is `a - 1` and the index is `i + 1`

#### Scenario: Braces inside parentheses restore newline sensitivity
- **WHEN** parsing `g({ a\n-1 })`
- **THEN** the closure body has two statements, `a` and `-1`

#### Scenario: Leading parenthesis keeps its existing rule
- **WHEN** parsing `cleanup()\n(1 + 2).toString()`
- **THEN** `(1 + 2).toString()` is a separate statement, as before

### Requirement: Separators Between Statements and Members
Two statements in a block or closure body MUST be separated by a newline or `;`. Two struct fields, enum variants or `when` branches MUST be separated by a newline, `,` or `;`. The parser MUST reject a missing separator with `E2001`.

#### Scenario: Statements on one line need a separator
- **WHEN** parsing `fun f() { val a = 1 val b = 2 }`
- **THEN** the parser reports `E2001` asking for a newline or `;` between statements

#### Scenario: Semicolon separates statements on one line
- **WHEN** parsing `fun f() { val a = 1; val b = 2 }`
- **THEN** the body has two statements

#### Scenario: Struct fields and enum variants on one line need a separator
- **WHEN** parsing `struct P { val x: Int val y: Int }` or `enum E { A B }`
- **THEN** the parser reports `E2001`, while `struct P { val x: Int, val y: Int }` and `enum E { A, B }` parse

#### Scenario: When branches on one line need a separator
- **WHEN** parsing `when (x) { 0 -> a 1 -> b }`
- **THEN** the parser reports `E2001`, while `when (x) { 0 -> a, 1 -> b }` parses

### Requirement: Blocks Hold Statements and an Optional Tail
A block or closure body MUST parse into a sequence of statements followed by an optional tail expression, which gives the block its value. The tail MUST be the block's last item when that item is an expression; a block ending in a declaration, an assignment or a loop, and an empty block, have no tail. A `;` after the last expression does not change this.

#### Scenario: Block ending in an expression
- **WHEN** parsing `fun f(): Int { val a = 1; a + 1 }`
- **THEN** the body has one statement, `val a = 1`, and the tail `a + 1`

#### Scenario: Block ending in an assignment
- **WHEN** parsing `fun f() { var x = 0; x = 1 }`
- **THEN** the body has two statements and no tail

#### Scenario: Trailing semicolon keeps the tail
- **WHEN** parsing `fun f(): Int { 1; }`
- **THEN** the body has no statements and the tail `1`

### Requirement: Assignment Is a Statement
An assignment, with `=` or a compound operator such as `+=`, MUST be a statement, never an expression. It MAY appear as a statement in a block or closure body, or as the unbraced body of an `if`/`else` branch, a `when` branch, or a `while` or `for` loop, where it forms a block with no tail. Anywhere else the parser MUST report `E2001`.

#### Scenario: Assignments as unbraced branch bodies
- **WHEN** parsing `if (c) x = 1 else x = 2` and `when (n) { 0 -> x = 3, else -> x = 4 }` inside a function body
- **THEN** each branch body is a block holding the assignment, with no tail

#### Scenario: Parenthesized assignment as a value rejected
- **WHEN** parsing `fun f() { var x = 0; val y = (x = 1) }` or `fun f() { var x = 0; g((x = 1)) }`
- **THEN** the parser reports `E2001`, while `g(x = 1)` still parses as a named argument

### Requirement: Compound Assignment
A statement MAY be a compound assignment `target op= value`, where `op=` is one of `+=`, `-=`, `*=`, `/=` and `%=`, each a single token. Its target MUST be one that `=` accepts: a variable, a `.` field access, an index expression or a write through `*`. Any other target, a chained assignment, and a compound operator in expression position MUST be rejected with `E2001`. A newline before the operator MUST end the previous expression.

#### Scenario: Compound assignment statements
- **WHEN** parsing the statements `x += 1`, `x -= 1`, `x *= 2`, `x /= 2`, `x %= 2`, `u.count += 1`, `xs[i] += 1`, `*r += 1` and `count += 1` inside a function body
- **THEN** each parses as a compound assignment of its operator, with the written target and the value as its right-hand side

#### Scenario: Compound value binds looser than every operator
- **WHEN** parsing `x += a ?: b * c` and `*r.n += 1`
- **THEN** the value of the first is the whole expression `a ?: (b * c)`, and the target of the second is `*` applied to `r.n`

#### Scenario: Compound assignment in an unbraced body
- **WHEN** parsing `if (c) x += 1 else x -= 1`, the `when` branch `0 -> x += 1` and `while (c) x *= 2`
- **THEN** each body is a block holding the compound assignment, with no tail

#### Scenario: Invalid compound assignment target rejected
- **WHEN** parsing `a + b += 1`, `1 += 2`, `g() += 1` or `a?.b += 1`
- **THEN** the parser reports `E2001` at the target, as it does for `=`

#### Scenario: Compound assignment as a value rejected
- **WHEN** parsing `fun f() { var x = 0; val y = (x += 1) }`, `fun f() { var x = 0; g(x += 1) }`, `fun f() { var x = 0; val y = x += 1 }` or `fun f() { var a = 0; var b = 0; a += b += 1 }`
- **THEN** the parser reports `E2001` at the `+=` inside the parentheses, in the argument, after `val y = x`, and at the second `+=`, saying that an assignment is a statement or that chained assignments are not allowed

#### Scenario: A newline before a compound operator ends the expression
- **WHEN** parsing `fun f() {\nvar x = 0\nx\n+= 1\n}`
- **THEN** the parser reports `E2001` at `+=`, because the statement `x` ends at the newline and no expression starts with `+=`

#### Scenario: Compound operators are single tokens
- **WHEN** tokenizing `a+=b`, `a-=b`, `a*=b`, `a/=b` and `a%=b`, and `a = -b`, `a->b`, `a == b` and `a /* c */ = b`
- **THEN** each of the first five yields an identifier, one compound operator token and an identifier, and the others keep their existing tokens

### Requirement: No Increment or Decrement Operators
The lexer MUST read `++` and `--` as single tokens, and the parser MUST reject either anywhere with `E2001`, with help suggesting the compound assignment `x += 1` or `x -= 1`. A double negation or the subtraction of a negation MUST be written with a space or parentheses.

#### Scenario: Increment and decrement rejected
- **WHEN** parsing the statements `x++`, `x--`, `++x` and `--x` inside a function body
- **THEN** the parser reports `E2001` at the `++` or `--`, with help suggesting `x += 1` or `x -= 1`

#### Scenario: Double minus needs a space
- **WHEN** parsing `fun f(a: Int, b: Int): Int = a--b`, `fun g(a: Int, b: Int): Int = a - -b` and `fun h(x: Int): Int = -(-x)`
- **THEN** the parser reports `E2001` at `--` in `f`, and accepts `g` and `h`

### Requirement: Half-Open Range Operator
The lexer MUST read `..<` as one token, and the parser MUST give it the precedence of `..`. `a..<b` denotes the range from `a` up to but excluding `b`. A newline before `..<` ends the expression, as for `..`.

#### Scenario: Half-open range tokens
- **WHEN** tokenizing `4..<10`
- **THEN** the tokenizer emits `Int(4)`, `DotDotLess` and `Int(10)`, while `4..10` still emits `Int(4)`, `DotDot` and `Int(10)`

#### Scenario: Half-open range precedence
- **WHEN** parsing `0..<n - 1`
- **THEN** arithmetic binds tighter than the range: `(0 ..< (n - 1))`

#### Scenario: Half-open range in a when pattern
- **WHEN** parsing `when (x) { in 0..<10 -> "small", else -> "large" }`
- **THEN** the first branch's pattern is an `in` range that excludes its end

### Requirement: Reserved Loop Keywords
`while`, `do`, `for`, `break` and `continue` MUST be reserved keywords. Using one as an identifier (a variable, parameter, function, field or named argument) MUST be rejected with `E2001`.

#### Scenario: Loop keyword as a variable name rejected
- **WHEN** parsing `fun f() { val for = 1 }`
- **THEN** the parser reports `E2001`

### Requirement: Copy and Clone Markers on Struct and Enum Declarations
A struct or enum declaration MUST accept an optional marker, `: Copy` or `: Clone`, in Kotlin supertype position: after the name and type parameters, after a shorthand struct's parameter list, and before the body. Exactly one of the names `Copy` and `Clone` MUST follow the colon; anything else MUST be rejected with `E2001`. Both names stay ordinary identifiers everywhere else.

#### Scenario: Markers on a block struct and an enum
- **WHEN** parsing `struct Point : Copy { val x: Int, val y: Int }`, `struct User : Clone { val name: String }` and `enum Color : Copy { Red, Green }`
- **THEN** the declarations parse, marked `Copy`, `Clone` and `Copy` respectively

#### Scenario: Marker after type parameters and a shorthand parameter list
- **WHEN** parsing `struct Tagged<T> : Copy { val id: Int }` and `struct Name(val v: String) : Clone`
- **THEN** both declarations parse with their markers

#### Scenario: Declaration without a marker
- **WHEN** parsing `struct Point { val x: Int }`
- **THEN** the declaration parses with no marker

#### Scenario: Other names after the colon rejected
- **WHEN** parsing `struct P : Eq { val x: Int }`, `struct P : Copy, Clone { val x: Int }`, `struct P : Copy, Copy { val x: Int }` or `struct P : { val x: Int }`
- **THEN** the parser reports `E2001`, saying that exactly one of `Copy` and `Clone` may follow `:` in a struct or enum declaration, and that `Copy` already implies `Clone`

#### Scenario: Marker in the wrong place rejected
- **WHEN** parsing `struct P : Copy <T> { val x: T }` or `struct Id : Copy (val v: Int)`
- **THEN** the parser reports `E2001`

#### Scenario: Marker names remain identifiers
- **WHEN** parsing `fun f(Copy: Int, Clone: Int): Int = Copy + Clone` and `struct Clone { val x: Int }`
- **THEN** both declarations parse

### Requirement: Writing Through a Reference
An assignment target MAY start with one or more prefix `*` operators followed by an operand parsed at prefix precedence, so postfix operators bind tighter (`*r.x = 1` is `*(r.x) = 1`). A `*` that starts any other expression MUST be rejected with `E2001`, with a message saying that `*` is only allowed on the left of `=` and that reads through references are implicit. A newline before `*` MUST end the previous expression.

#### Scenario: Write-through statement
- **WHEN** parsing the statements `*r = 5`, `**rr = 0` and `*pick(&mut a, &mut b) = 1`
- **THEN** each parses as an assignment whose target is `r`, `rr` or the call, under one `*` per prefix operator written

#### Scenario: Postfix binds tighter than prefix star
- **WHEN** parsing `*r.x = 1`
- **THEN** the assignment target is `*` applied to `r.x`

#### Scenario: Prefix * is not a binary operator after a newline
- **WHEN** a statement `*r = 5` follows a line ending in an expression (`val a = b` then `*r = 5`)
- **THEN** it parses as a new statement, by the existing rule that a newline before a binary operator ends the expression

#### Scenario: Write-through in an unbraced branch or loop body
- **WHEN** parsing `if (c) *r = 1 else *r = 2`, the `when` branch `0 -> *r = 1` and `while (c) *r = 0`
- **THEN** each body is an assignment through `r`

#### Scenario: Prefix star outside an assignment target rejected
- **WHEN** a `*` starts an expression that is not an assignment target (`val x = *r`, `g(*r)`, `return *r`, `(*r).x = 1`, or the statement `*r` with no `=`)
- **THEN** the parser reports `E2001` at the `*`, saying that `*` writes through a reference only on the left of `=` and that reads through references are implicit

#### Scenario: Multiplication unchanged
- **WHEN** parsing `a * b` and `a *b` on one line
- **THEN** both parse as multiplication

### Requirement: Extension Function Declarations
A function declaration MAY name a receiver type before its name, `fun T.name(...)`, which declares an extension function on `T`. The receiver MUST be a single type name, optionally with type arguments (`fun <T> Box<T>.get()`, `fun Box<Int>.sum()`). A receiver that is one of the function's own type parameters, a `Heap<...>` receiver, a `?`, a `&` or `&mut`, or a parenthesized or function type MUST be rejected with `E2001`, with a message saying what receivers may be.

#### Scenario: Extension declarations
- **WHEN** parsing `fun User.greet(): String = "Hi " + name`, `public fun Int.squared(): Int = this * this` and `public compose fun User.Avatar(): Unit { }`
- **THEN** each parses as a function named `greet`, `squared` or `Avatar` with the receiver type `User`, `Int` or `User` and the shared receiver mode

#### Scenario: Receivers with type arguments
- **WHEN** parsing `fun <T> Box<T>.get(): &T = &v` and `fun Box<Int>.sum(): Int = v`
- **THEN** each parses as an extension whose receiver type is `Box<T>` or `Box<Int>`

#### Scenario: Receiver with type arguments rejected
- **WHEN** parsing `fun Heap<User>.size(): Int = 0`
- **THEN** the parser reports `E2001`, saying that calls look through `Heap`, so the extension is declared on its contents (`fun User.size()`)

#### Scenario: Type-parameter receiver rejected
- **WHEN** parsing `fun <T> T.also(): T = this`
- **THEN** the parser reports `E2001`, saying that a bare type parameter cannot be a receiver

#### Scenario: Nullable or borrowed receiver rejected
- **WHEN** parsing `fun User?.greet(): String = "x"`, `fun &User.greet(): String = "x"` or `fun &mut User.rename() { }`
- **THEN** the parser reports `E2001`; for `User?` the message says to declare the extension on `User` and call it with `?.`, and for `&` it says that the receiver mode (`fun`, `mut fun`, `consuming fun`) decides how the receiver is borrowed

#### Scenario: Qualified receiver rejected
- **WHEN** parsing `fun models.User.greet(): String = "x"`
- **THEN** the parser reports `E2001`, since type names are not qualified

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

### Requirement: The this Expression
`this` MUST parse as an expression atom wherever an expression may start, with postfix operators applying to it (`this.name`, `this.greet()`, `&this`). `this` MUST stay a reserved keyword that cannot name a variable, parameter, function, field or named argument.

#### Scenario: this in expressions
- **WHEN** parsing the function bodies `this * this`, `this.name`, `this.greet()` and `use(&this)`
- **THEN** each parses, with `this` as the operand, the target of the member access or call, or the operand of `&`

#### Scenario: this as a name rejected
- **WHEN** parsing `fun f() { val this = 1 }` or `fun g(this: Int) { }`
- **THEN** the parser reports `E2001`

### Requirement: Type Parameter Lists
A function MUST declare its type parameters between `fun` and its name or receiver (`fun <T> id(x: T)`, `fun <T> Box<T>.get()`), and a struct or enum after its name. A list MUST hold one or more names, optionally with a trailing comma. A function type parameter MAY have a bound, `: Copy` or `: Clone`. Type parameters after a function's name, an empty list, any other bound, and a bound in a struct or enum MUST be rejected with `E2001`.

#### Scenario: Type parameter lists
- **WHEN** parsing `fun <T> id(x: T): T = x`, `public compose fun <T : Copy, U : Clone,> show(a: T, b: U) {}`, `mut fun <T> Box<T>.put(v: T) {}`, `struct Pair<A, B>(val a: A, val b: B)` and `enum Option<T> : Copy { Some(T), None }`
- **THEN** each declaration parses with its type parameters in order, `T` bounded by `Copy` and `U` by `Clone` in `show`

#### Scenario: Type parameters after the function name rejected
- **WHEN** parsing `fun id<T>(x: T): T = x` or `fun User.f<T>(x: T) {}`
- **THEN** the parser reports `E2001`, saying that type parameters go before the name, as in `fun <T> id(...)`

#### Scenario: Malformed type parameter lists rejected
- **WHEN** parsing `fun <> f() {}`, `fun <T : Eq> f(x: T) {}`, `fun <T : Copy, Clone> f(x: T) {}` or `struct Box<T : Copy>(val v: T)`
- **THEN** the parser reports `E2001`: a list needs a type parameter, only `Copy` or `Clone` can bound one, and struct and enum type parameters cannot be bounded yet

### Requirement: Explicit Type Arguments at Call Sites
After an identifier or a member name in an expression, `<` MUST start a type-argument list of the call when the tokens up to the matching `>` parse as types, the list closes with `>`, and the next token is a `(` or `{` on the same line. Otherwise `<` MUST be the comparison operator. In `g(a < b, c > (d))`, the type-argument reading MUST win.

#### Scenario: Type arguments on calls
- **WHEN** parsing `id<Int>(1)`, `Box<String?>(null)`, `remember<Int> { 0 }`, `s.map<Box<Int>> { n -> Box(n) }` and `u?.pick<Int>(1)`
- **THEN** each parses as a call with the written type arguments, and `Box<Box<Int>>` closes with two `>` tokens

#### Scenario: Comparisons stay comparisons
- **WHEN** parsing `a < b`, `if (i<n) f()`, `val m = a < b + 1` and `val k = a < b >= c`
- **THEN** the first three are comparisons, and the last is rejected with `E2001` as a chained comparison, because a list closed by the `>` of `>=` is not a type-argument list

#### Scenario: Comma-separated comparisons read as type arguments
- **WHEN** parsing `g(a < b, c > (d))`
- **THEN** the call `g` has one argument, the call `a<b, c>(d)`, while `g((a < b), c > (d))` has two comparison arguments

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

### Requirement: Escaping Parameter Modifier
A parameter of a function declaration, or a parameter in a function type's parameter list, MAY be written with `escaping` before its type when that type is a function type, composable or not, or a parenthesized nullable function type. `escaping` before any other type, or in any other type position, MUST be rejected with `E2001`, saying that `escaping` marks function-typed parameters.

#### Scenario: Escaping parameters
- **WHEN** parsing `fun button(onClick: escaping () -> Unit, content: escaping compose () -> Unit) {}`, `fun later(f: escaping (() -> Unit)? = null) {}` and `struct Bus { val register: (escaping () -> Unit, Int) -> Unit }`
- **THEN** `onClick`, `content` and `f` are escaping parameters of types `() -> Unit`, `compose () -> Unit` and `(() -> Unit)?`, and the first parameter of the type of `register` is escaping while the second is not

#### Scenario: Escaping on another type rejected
- **WHEN** parsing `fun f(n: escaping Int) {}`, `fun g(x: escaping (Int)) {}`, `fun h() { val k: escaping () -> Unit = {} }` or `fun m(): escaping () -> Unit = {}`
- **THEN** the parser reports `E2001` at `escaping` for each, saying that `escaping` marks parameters of function type

### Requirement: The Name it Is an Identifier
`it` MUST be an ordinary identifier, never a keyword: it MAY name a variable, parameter, closure parameter, function, field or argument, and a closure MAY declare a parameter named `it` in its header. The syntax tree MUST NOT add an implicit parameter to a closure; whether a closure has one is decided by the type checker.

#### Scenario: it as a declared name
- **WHEN** parsing `fun it(it: Int): Int = it`, `fun f() { val it = 1; g(it = it) }`, `x.it()` and `run { it -> it + 1 }`
- **THEN** each `it` parses as a function, parameter, variable, argument, member or closure parameter name, and the closure has the one declared parameter `it`

#### Scenario: A header-less closure has no parameters in the syntax tree
- **WHEN** parsing `xs.forEach { println(it) }`
- **THEN** the closure has no header and no parameters, and `it` in its body is an identifier expression

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

### Requirement: Receivers of Function Types
The receiver of a function type MUST be a single type name, optionally with type arguments, and MAY be a type parameter in scope. A nullable, `&` or `&mut`, parenthesized, function or `Heap<...>` receiver MUST be rejected with `E2001`, with the message an extension receiver of that form gets, and `consuming` before a receiver MUST be rejected with `E2001`, saying that a function type's receiver is shared or `mut`.

#### Scenario: Receiver forms
- **WHEN** parsing `fun <T> with(receiver: &T, block: T.() -> Unit) { }`, `fun f(a: List<Int>.(Int) -> Bool) { }` and `fun g(h: (ColumnScope.() -> Unit)?) { }`
- **THEN** the receivers are the type parameter `T` and `List<Int>`, and `h` is a nullable function type whose receiver is `ColumnScope`

#### Scenario: Unsupported receivers rejected
- **WHEN** parsing `fun f(a: User?.() -> Unit) { }`, `fun g(a: &User.() -> Unit) { }`, `fun h(a: Heap<User>.() -> Unit) { }` or `fun k(a: consuming User.() -> Unit) { }`
- **THEN** the parser reports `E2001` for each

### Requirement: Context Parameter Declarations
A top-level function declaration MAY start with one context parameter, `context(name: Type)`, before its visibility and every other modifier. The type MUST follow the rules of an extension receiver and MUST NOT mention the declaration's own type parameters. A second context parameter, an empty `context()`, a context parameter without a name or type, and `context(...)` before a struct, enum or `val` declaration or before an `external` or `intrinsic` function MUST be rejected with `E2001`.

#### Scenario: Context parameter declarations
- **WHEN** parsing `context(scope: RowScope) public fun Modifier.weight(weight: Double, fill: Bool = true): Modifier = this`, `context(scope: ColumnScope) compose fun Divider() { }` and `context(s: RowScope) fun <T> Box<T>.cell(): Int = 1`
- **THEN** each parses as a function with the context parameter `scope` of type `RowScope`, `scope` of type `ColumnScope` or `s` of type `RowScope`, with its visibility, `compose` flag and receiver as written

#### Scenario: Invalid context parameters rejected
- **WHEN** parsing `context(a: RowScope, b: ColumnScope) fun f() { }`, `context() fun g() { }`, `context(RowScope) fun h() { }`, `context(s: RowScope) struct S { }`, `context(s: RowScope) val v = 1`, `context(s: RowScope) private external fun e(): Int`, `public context(s: RowScope) fun m() { }` or `context(s: Box<T>) fun <T> k() { }`
- **THEN** the parser reports `E2001` for each

### Requirement: Intrinsic Struct Declarations
`intrinsic struct`, optionally after a visibility modifier, MUST declare a struct without fields or constructor: its name, an optional type parameter list and optional markers (`public intrinsic struct RecomposeScope : Copy`), and no parenthesized fields and no block. An intrinsic struct with fields or a block, and a struct with both `intrinsic` and `external`, MUST be rejected with `E2001`. Which files may use it is the rule of `standard-library`.

#### Scenario: Intrinsic structs parse
- **WHEN** parsing `public intrinsic struct Composition` and `public intrinsic struct RecomposeScope : Copy`
- **THEN** each parses as an intrinsic struct declaration with its visibility, name and markers

#### Scenario: An intrinsic struct with fields rejected
- **WHEN** parsing `intrinsic struct A(val n: Int)`, `intrinsic struct B { val n: Int }` or `external struct C`
- **THEN** the parser reports `E2001`

### Requirement: Intrinsic Writable Properties
`intrinsic var`, optionally after a visibility modifier, MUST declare an extension property as `intrinsic val` does, with an optional type parameter list, a receiver type, `.`, the name, `:` and its type, and no initializer. A `var` declaration with a receiver and without `intrinsic`, and an `intrinsic var` with an initializer, MUST be rejected with `E2001`.

#### Scenario: A writable intrinsic property parses
- **WHEN** parsing `public intrinsic var <T> MutableState<T>.value: T`
- **THEN** it is an intrinsic extension property marked writable, with type parameter `T`, receiver `MutableState<T>`, name `value` and type `T`

#### Scenario: Writable extension properties need intrinsic
- **WHEN** parsing `var Int.twice: Int` or `intrinsic var Int.twice: Int = 2`
- **THEN** the parser reports `E2001`

### Requirement: Attached Parameter Modifier
A parameter of a function declaration, or a parameter in a function type's parameter list, MAY be written with `attached` before its type where `escaping` may be written. `attached` before any other type, in any other type position, or together with `escaping`, MUST be rejected with `E2001`, saying that `attached` marks function-typed parameters that the composition keeps.

#### Scenario: Attached parameters
- **WHEN** parsing `fun Button(onClick: attached () -> Unit, content: compose () -> Unit) {}`, `fun runApp(content: attached compose () -> Unit) {}`, `fun later(f: attached (() -> Unit)? = null) {}` and `fun register(r: (attached (Bool) -> Unit) -> Int) {}`
- **THEN** `onClick`, `content` of `runApp` and `f` are attached parameters of types `() -> Unit`, `compose () -> Unit` and `(() -> Unit)?`, and the parameter of the type of `r` is attached

#### Scenario: Attached on another type or with escaping rejected
- **WHEN** parsing `fun f(n: attached Int) {}`, `fun g() { val k: attached () -> Unit = {} }` or `fun h(f: escaping attached () -> Unit) {}`
- **THEN** the parser reports `E2001` at `attached` for each

### Requirement: Capture Function Modifier
`capture` MAY appear directly before `fun`, after `intrinsic`, in a declaration with a receiver, to mark the function that closures given to `attached` parameters use to capture a value of the receiver's type. `capture` without `intrinsic`, without a receiver, or together with `compose`, `mut` or `consuming`, MUST be rejected with `E2001`.

#### Scenario: A capture function
- **WHEN** parsing `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>`
- **THEN** it is an intrinsic extension function marked as a capture function, with type parameter `T`, receiver `MutableState<T>` and name `weak`

#### Scenario: Capture outside its form rejected
- **WHEN** parsing `capture fun Int.weak(): Int = 1`, `public intrinsic capture fun weak(): Int` or `public intrinsic capture mut fun Int.weak(): Int`
- **THEN** the parser reports `E2001` for each

## Design Rationale
Prism chooses Kotlin-like ergonomics combined with value ownership semantics to eliminate GC pauses without lifetime annotation complexity. Decisions from `lexer-parser`:

- **Hand-written lexer, no generator.** A streaming lexer over UTF-8 bytes keeps the bootstrap compiler free of build dependencies and procedural macros (`logos` was rejected), and gives exact control over newline tracking (`is_preceded_by_newline`), nested comments and `4..10` versus `4.10`.
- **Pratt parsing for expressions, recursive descent for statements and declarations.** One precedence table defines every operator. LALR generators (`lalrpop`) were rejected because they struggle with trailing lambdas, contextual modifiers and semicolon-free newlines.
- **Elvis between comparison and range.** `?:` binds tighter than comparison and equality but looser than range and arithmetic, so `age ?: 0 >= 18` is `(age ?: 0) >= 18` and `base ?: offset + 10` is `base ?: (offset + 10)`.
- **Non-associative assignment.** `a = b = c` is rejected to rule out assignment-as-expression bugs.
- **Mandatory parentheses on `if` and `when`.** Without them, `if isValid { doWork() }` would parse `{ doWork() }` as a trailing lambda on `isValid`. Multi-token lookahead to tell the two apart was rejected as fragile.
- **Newlines end statements.** A `(` after a newline is not a call, and `return` before a newline, or before a token that cannot start an expression (`}`, `else`, `,`, `)`, end of file), returns unit. Mandatory semicolons were rejected for ergonomics.
- **Multi-statement closures.** Closure bodies are blocks, because single-expression closures cannot hold setup logic.
- **`compose` is a soft keyword.** It is a modifier only directly before `fun`, so common API names such as `pipeline.compose()` stay usable.
- **Borrowed struct fields are rejected in the parser.** Enforcing the zero-cost memory invariant at AST construction means no struct can ever carry a lifetime dependency. The same holds for enum payloads and for a borrow nested in `?` or a type argument (`(&String)?`, `Heap<&Int>`), which an earlier check that only looked at a field's outermost type let through. A function type's parameters and return type may mention borrows: a function value stores none, and borrows only during a call.
- **`package` declaration versus `package` modifier.** A lookahead (`is_package_decl_ahead`) treats `package` followed by a dotted name as a declaration and `package fun` (or `struct`, `enum`, `val`) as visibility.
- **A fixed nesting limit (256) instead of a growable stack.** Recursive descent overflowed the stack on deep input and aborted the process. A fixed limit keeps the diagnostic the same for every build of the compiler and bounds the tree for every later recursive pass; growing the stack would need a dependency with a build script.
- **Fail-fast errors.** The parser stops at the first syntax error with an exact span and never panics. Multi-error recovery is left for LSP support: it needs error nodes in the AST and synchronization points built on the newline and separator rules, and resynchronizing per declaration would change what `parse_source` returns (`harden-parser-grammar`).

Decisions from `harden-parser-grammar`:

- **A brace's meaning comes from its position, not from lookahead.** `{` is a block only as a function body, an `if`/`else` or `when` branch body, or at the start of a statement; everywhere else it is a closure, with zero parameters when it has no `->` header, and its body is always a block. Lookahead for a parameter header used to decide, so `run { 1 }` evaluated eagerly in the caller and `takesInt { 42 }` typechecked. Restricting closures to trailing-lambda and argument position was rejected because `val f = { 1 }` would then differ from `g({ 1 })`.
- **Statement-position braces are scopes, unlike Kotlin.** In Kotlin a statement `{ … }` is a lambda that is never called. Prism keeps it as a nested scope, the natural way to end a borrow early under the zero-cost memory invariant. As in Kotlin, `fun f() = { 1 }` returns a closure.
- **Declarations are statements.** `val`/`var` parse only directly inside a block or closure body, and a declaration's initializer is parsed above assignment precedence so `val a = b = c` is a chained assignment. Kotlin's unbraced declaration as a branch body was rejected because Prism's `if` is an expression and the declaration's scope would leak. (`statements-and-loops` replaced this change's choice to keep declarations inside `BlockExpr.stmts` with a `Statement` type; see below.)
- **Kotlin's newline rule for operators.** Outside `( )` and `[ ]`, a newline before a binary operator ends the expression unless the operator is `.`, `?.`, `&&`, `||` or `?:`; inside them newlines are insignificant, and a `{` restores sensitivity. Making only `+`/`-` sensitive was rejected: `*`, `==` and `..` would still silently join lines.
- **Separators between items on one line.** Statements need a newline or `;`, and struct fields, enum variants and `when` branches a newline, `,` or `;`, so `val a = 1 val b = 2` is an error rather than two statements; together with the newline rule, juxtaposition never silently means "two things".
- **Function types carry `compose`.** `compose (A) -> R` is a distinct type from `(A) -> R`, written in the ABI string, because a composable content parameter is a different call contract. `() -> Unit?` returns `Unit?`; a nullable function type is `(() -> Unit)?`, as in Kotlin.

Decisions from `statements-and-loops` (agreed with the user in the borrow-checker roadmap exploration):

- **A `Statement` AST type and an explicit tail.** A block is `statements` plus an optional `tail` expression that gives its value. Loops are statements that are not expressions, so the AST models statements directly instead of leaving them to a later typed IR. Making the tail explicit means later stages (the typed tree, MIR lowering) never re-derive "is the last item the value?". The parser decides the tail: a final expression statement is the tail whether or not a `;` follows it, as in Kotlin. Rust's rule that `;` discards the value was rejected because Kotlin users don't expect `;` to change meaning.
- **Assignment is a statement.** It's still allowed as an unbraced `if`/`else`, `when` or loop body, which keeps Kotlin forms such as `if (c) x = 1 else x = 2` working, as a one-statement block with no tail. Everywhere else, `=` in an expression is `E2001`. Keeping `Assign` as a `Unit`-typed expression was rejected because it would leave the statement-or-expression ambiguity in place.
- **`..<` for half-open ranges.** It's Kotlin's own operator (since 1.9), at the precedence of `..`. Closed ranges only (`0..n - 1`) were rejected as off-by-one prone. Kotlin's `until` is an infix function there and would have to be a special-cased keyword here.
- **Hard loop keywords.** `while`, `do`, `for`, `break` and `continue` are reserved, as in Kotlin. A soft keyword needs a following token to tell it apart, but `while (x)` would also read as a call.

Decisions from `value-semantics` (agreed with the user):

- **Markers in Kotlin supertype position, one name only.** `: Copy` or `: Clone` goes after the type parameters and a shorthand parameter list, before the body, where Kotlin writes supertypes and where a marker-trait list would go once traits exist. Only the identifiers `Copy` and `Clone` are accepted, exactly one, and neither is a keyword. A general supertype list was rejected: trait conformances are expected to need their own block, because methods are extension functions, so a list would invent rules for names that may never appear there. Widening the marker into a list later breaks no program.
- **Top-level declarations are `val` only.** `var` declares locals; a top-level `var` is `E2001`, with a message pointing to locals, parameters and state handles (see `ownership`).

Decisions from `reference-access`:

- **Prefix `*` only starts an assignment target** (user). `*r = v` writes through a reference, with one `Deref` per `*` (`**rr = v`) around an operand parsed at prefix precedence, so `*r.x = 1` is `*(r.x) = 1`. Anywhere else `*` is `E2001`, with a message saying reads through references are implicit, and `starts_expr` accepts it so that `return *r` reaches that message. A newline before `*` still ends the previous expression, so `*r = 5` on its own line is a new statement. A separate statement kind was rejected: `Deref` is the projection the typed tree and MIR places use, and it nests.

Decisions from `extension-methods`:

- **Receiver modes are modifiers in a fixed order** (user for the modifiers, order autonomous): `[visibility] [compose] [mut | consuming] fun Receiver.name`. The common case stays unmarked and the forms follow Swift's `mutating` / `consuming`; Rust's explicit `&self` / `&mut self` / `self` parameter was the alternative. A fixed order buys a simpler grammar and formatter at no cost with three modifiers. The mode lives in the receiver node, so a mode without a receiver cannot be represented.
- **`consuming` is a contextual keyword, like `compose`.** It is a modifier only directly before `fun` at the start of a declaration, so no existing name breaks; a hard keyword would have bought nothing grammatically.
- **A receiver is a single type name for now.** Type arguments came with `generics` (see its decisions below), a nullable receiver is declared on the non-null type and called with `?.`, and a reference receiver is expressed by the mode. Each is `E2001` with a message saying so, and lifting any of them only accepts more programs. When qualified type paths arrive, the last `.` segment before the name is the method name, as in Kotlin.
- **`this` is a reserved keyword and its own expression node,** so the formatter and LSP can tell it apart from a name, and no declaration can be called `this`.

Decisions from `generics` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Type parameters go between `fun` and the name or receiver** (`fun <T> id`, `fun <T> Box<T>.get()`), as in Kotlin, because a generic receiver mentions them and must not read `T` before declaring it. Structs and enums keep `struct Box<T>`. The old `fun id<T>` placement is `E2001` with a hint. `Copy` and `Clone` cannot name a type parameter, so `<T : Copy, Clone>` is a rejected second bound rather than a parameter named `Clone`. Rejected: Rust/Swift's `fun id<T>` plus a second placement for extensions.
- **Call type arguments use Kotlin's syntax with a deterministic rule.** After an identifier or member name, `<` is tried as a type-argument list, kept when the tokens up to the matching `>` parse as types, the list closes with `>` (never the `>` of `>=`), and a `(` or `{` follows on the same line; otherwise the parser rewinds and `<` is a comparison. Comparisons are non-associative, as in Kotlin's grammar (`a < b < c` never typechecked), which leaves `g(a < b, c > (d))` as the only double reading; it is a generic call, as in Kotlin and C#, the way `else` binds to the nearest `if`. Rejected: Rust's turbofish (foreign to `mutableStateOf<String?>(null)`, and it takes `::` from method references), whitespace-sensitive `<`, and name-directed parsing, which would make the parser depend on the type checker.
- **A receiver may carry type arguments, but not be a bare type parameter or `Heap<...>`.** `fun <T> T.also()` would make every extension lookup generic, and a `Heap` receiver competes with looking through `Heap` layers.

Decisions from `compound-assignment` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Compound assignment is a statement with the targets of `=`.** `+=`, `-=`, `*=`, `/=` and `%=` are single tokens, accepted wherever `=` is, with the same target check, same-line rule and chained-assignment error; in expression position they reach the "an assignment is a statement" error, because a compound assignment is an assignment. The AST has its own `CompoundAssign` statement rather than an optional operator on `Assign`, so every consumer (type checker, ABI body hash, formatter) must decide what it means instead of silently treating it as `=`.
- **`++` and `--` are reserved tokens that are always rejected.** The lexer reads them as Kotlin's does and the parser reports `E2001` with help suggesting `x += 1`, in prefix and postfix position. Kotlin's `++` expressions were rejected because their value (old or new, by position) makes an assignment an expression, against "Assignment Is a Statement"; a statement-only `x++` because it still invites `val y = x++` (Swift removed both, SE-0004); leaving the lexer alone because `a--b` would keep meaning `a - (-b)`, a reading Kotlin does not have. **Breaking:** `a--b` and `--x` need a space or parentheses. Reserving the tokens keeps adding the operators later non-breaking.
- **Bitwise operators are deferred.** Their spelling has real ambiguity costs (`&` is the borrow operator and `&&x` is two borrows, `|` is a lexer error, `>>` closes nested type arguments) and depends on sized integer types; Kotlin uses named infix functions (`and`, `shl`). A `bitwise-operators` change decides both and extends the compound token set (ROADMAP).

Decisions from `prism-fmt` (planned autonomously on 2026-10-17 and logged in `docs/decisions-for-review.md`):

- **Comments come from an opt-in lexer mode** (D2). `Lexer::tokenize_with_comments` returns the token stream of normal lexing plus every `Comment { kind, span }` (the text is the span's slice of the source); the lexer pushes to an optional list in `skip_whitespace_and_comments` only when collection is on, so compilation pays one untaken branch per comment. The formatter lexes once in this mode and hands the tokens to the parser, so its tree and comments come from the same scan. Rejected: trivia on every token (Roslyn, rust-analyzer), which grows every token for one tool and is right only for a lossless syntax tree (a follow-up, see `formatting`); re-scanning the gaps between tokens, which would duplicate the nested block-comment rules; comments as AST nodes, which every later stage would have to skip.

Decisions from `string-templates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **Kotlin's `$name` and `${expression}`, and when `$` is literal** (D2). `$` followed by an ASCII letter or `_` begins a name part, the longest run of identifier characters (`"$a.b"` inserts `a`, then `.b`); `$` followed by `{` an expression part; any other `$` is text (`"$5"`, `"$"`, `"$$x"`, `"$é"`), and `\$` writes a `$` in string and `Char` literals. A name part is an identifier (including the soft keyword `compose`) or `this`; any other keyword (`"$if"`, `"$null"`) is `E2001` naming `\$if` and `${...}`, so a `$name` part is exactly the expression `name`. Relaxing that to text later breaks nothing. Rejected: Swift's `\(expression)`, JavaScript's backquotes, and braces only (`"${x}"`), which breaks the common Kotlin form.
- **Flat template tokens and a lexer mode stack** (D3). A template is a flat token run (start, text runs, `$name` parts, `${` and its `}` around ordinary tokens, end), as Kotlin's lexer does, so the parser stays a parser over one token vector and the formatter and future tools find tokens by offset. The lexer keeps a stack of `Template` and `TemplateExpression { open_braces }` modes; brace counting is exact because braces in literals and comments are never tokens. A literal without parts stays one `String` token, and the text before a literal's first part is scanned twice, once. A raw newline in template text and the end of the file end the open modes with one `Error` token; an unknown escape or a keyword after `$` is one `Error` token after which the template continues, so its closing quote still closes it. Lexing stays linear. Rejected: a token holding nested token vectors per part, re-lexing a substring per part (loses spans and comments), and a parser driving the lexer.
- **An expression part holds any expression** (D4), parsed with newlines insignificant as inside parentheses, so it may span lines and hold closures, `when`, nested templates and comments, as in Kotlin. Only the text between parts rejects a raw newline. Rejected: forbidding newlines or comments inside `${}`.
- **One tree node for every template** (D5). `ExprKind::Template` holds text and expression parts; a name part is an `Ident` (or `This`) spanning the name, so `"$x"` and `"${x}"` give the same tree and the formatter reads which was written from the tokens. A template is an atom that postfix operators apply to, never a place, and the span-free body hash covers it, so editing a template in a generic body changes the hash.

Decisions from `escaping-closures` (planned autonomously on 2026-10-18, revised for compile speed on 2026-10-19, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`escaping` is a contextual modifier at the start of a parameter's function type** (D3), only directly before `(` or `compose`, so `escaping` stays usable as a name and even as a type name; before any other type, or in any other type position, it is `E2001`, since a local, field or result already owns its value. The bare-modifier spelling follows `compose`, as Prism has no attribute syntax.

Decisions from `implicit-it` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **The tree records a closure's header** (D2). `ExprKind::Closure` holds `arrow: Option<Span>`, the header's `->`, so `{ -> body }` declares zero parameters and never gets an implicit `it`, while `{ body }` has no header and gets one against a one-parameter type, as in Kotlin. A span rather than a flag serves the formatter's comment placement and a future LSP, and the span-free body hash covers whether a closure has a header. Rejected: reading `{ -> body }` as `{ body }`, which leaves no way to say "this closure takes nothing"; `params: Option<Vec<ClosureParameter>>`, which changes the meaning of an existing field.
- **`it` is an ordinary identifier** (D3): no token, keyword or reserved name, so `val it`, `fun it`, a field or parameter `it` and `{ it -> ... }` stay valid, and the parser never adds a parameter (it does not know the expected type). Rejected: a contextual keyword, which needs a rule for every place a name may appear and buys nothing, since lookup is already lexical.

Decisions from `receiver-lambdas` (planned autonomously on 2026-10-19 for `ui-toolkit`'s Compose-like layout scopes, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`T.(A) -> R` and `mut T.(A) -> R`** (D1). A function type may name a receiver before its parameter list, Kotlin's syntax; `mut` before it makes the receiver mutable, mirroring the `mut fun` modifier of extensions, since Kotlin has no borrows. The prefixes keep the order of declarations (`escaping compose mut Builder.() -> Unit`), and the parser decides with one token of lookahead: a named type followed by `.(` is a receiver, and `mut` cannot otherwise start a type. The receiver follows the rules of extension receivers, except that a type parameter is allowed (`fun <T> with(receiver: &T, block: T.() -> Unit)`), because a lambda checked against `T.() -> R` is checked once `T` is known. Rejected: only shared receivers (builders such as `buildList { add(x) }` need to mutate theirs), `(&mut T).() -> R` (references are no receivers anywhere), and `consuming T.() -> R` for now (no use case; a follow-up).
- **`context(name: T)` declarations, one parameter, before every modifier** (D6). Kotlin 2.2's context parameters, limited to one, on any function with a body; the parameter is named, typed like a receiver and may not mention the declaration's own type parameters. `context` is a contextual keyword only at a top-level declaration's start directly followed by `(`, so it stays an identifier everywhere it was one. Rejected: `fun RowScope.weight(m: Modifier, ...)` or an explicit scope argument (unlike Compose's `Modifier.weight(1f)`), Kotlin's member extensions (Prism has no members), several context parameters (nothing needs them while contexts resolve to the innermost receiver; a follow-up), and context parameters on `external` and `intrinsic` declarations (a follow-up).

Decisions from `compose-transform` (planned autonomously on 2026-10-18, reconciled with `compose-runtime`, `receiver-lambdas` and `compose-hot-reload` and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **`intrinsic struct` declarations** (D16): visibility, `intrinsic struct`, a name, optional type parameters and markers, and neither fields nor a block, since the compiler provides the representation; `intrinsic` with fields or a block, or with `external`, is `E2001`. Which files may use the form is `standard-library`'s rule.

Decisions from `state-handles` (planned autonomously on 2026-10-18, reconciled with the archived compose changes and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **`intrinsic var` properties** (D4). The property form of `intrinsic val` also takes `var`, with a `writable` flag in the AST and the ABI, for `MutableState<T>.value`; `var` with a receiver and no `intrinsic`, or an `intrinsic var` with an initializer, is `E2001`.

Decisions from `owned-state` (review items A2′, A2‴ and L5 of A4, agreed with the user on 2026-10-20 in `docs/milestone-2-review-notes.md`: `MutableState<T>` an owning state object as in Compose, no stale-state errors in UI code, weak capture declared by the library; the rest decided while proposing, implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`attached` and `capture` are contextual modifiers** (D9, D12). `attached` is parsed where `escaping` is, on function-typed parameters, never both; `capture` directly before `fun`, after `intrinsic`, on an extension; both stay identifiers elsewhere (`E2001` on misuse).
