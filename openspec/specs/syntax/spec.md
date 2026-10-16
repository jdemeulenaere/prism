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
Variables MUST be immutable by default using `val`, while mutable variables MUST be explicitly declared with `var`.

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
- **WHEN** a `when` expression matches over values or ranges (`when (x) { 0 -> "zero", in 1..10 -> "small", else -> "large" }`)
- **THEN** each branch parses into its pattern (value, comma-separated values, `in` range, or `else`) and branch expression, in source order

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
The compiler MUST tokenize source text into a stream of typed tokens with accurate source spans, processing whitespace, line comments, nested block comments, and escape sequences while rejecting raw unescaped newlines in single-line strings.

#### Scenario: Columns count characters
- **WHEN** non-ASCII text precedes a token on its line (`/* café */ fun`)
- **THEN** the token's column counts characters, not UTF-8 bytes (`fun` is at column 12)

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
- **WHEN** a string or char literal contains a backslash followed by a character other than `n`, `r`, `t`, `\`, `"`, `'`, `0` or `u` (`"\q"`, `'\é'`)
- **THEN** tokenizer emits an `Error` token naming the unknown escape, without panicking on multi-byte characters

#### Scenario: Empty char literal rejected
- **WHEN** a char literal has no character (`''`)
- **THEN** tokenizer emits an `Error` token

#### Scenario: Unterminated string literal error
- **WHEN** a string literal is unclosed before end-of-file
- **THEN** tokenizer emits an `Error` token with the unterminated literal span without panicking

#### Scenario: Joined operator split where the grammar needs its parts
- **WHEN** a generic argument list is closed directly before `=` (`val y: Heap<Int>= x`), or `&&` appears where a prefix borrow is expected (`&&x`, `&&Int`)
- **THEN** the parser reads `>=` as `>` followed by `=`, and `&&` as two borrows, while `a && b` stays a logical and

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
The parser MUST parse expressions according to Pratt operator precedence, correctly ordering assignment, Elvis, logical, comparison, range, arithmetic, prefix, postfix, and function call operators.

#### Scenario: Arithmetic and comparison precedence
- **WHEN** parsing expression `a + b * c > d`
- **THEN** operator precedence structures AST as `((a + (b * c)) > d)`

#### Scenario: Non-associative assignment
- **WHEN** code attempts chained assignment (`a = b = c`)
- **THEN** the parser rejects the expression with a compile error requiring separate statements

#### Scenario: Invalid assignment target rejected
- **WHEN** the left side of `=` is not a variable, a `.` field access or an index expression (`a + b = c`, `1 = 2`, `g() = 1`, `a?.b = c`)
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
Struct declarations MUST NOT declare fields with borrowed reference types (`&T` or `&mut T`), enforcing Invariant 4 at parse time.

#### Scenario: Struct field declaring borrowed reference rejected
- **WHEN** a struct definition declares a field with reference type (`struct View { val data: &String }`)
- **THEN** the parser emits a compile error rejecting borrowed references in structs

### Requirement: Contextual Keyword Identifier Permissibility
Modifiers such as `compose` MUST function contextually, permitting their use as regular identifiers in member access and parameter declarations.

#### Scenario: Member access on compose identifier
- **WHEN** code accesses a property or method named compose (`pipeline.compose(step)`)
- **THEN** `compose` is parsed as a valid identifier expression rather than a modifier keyword

#### Scenario: Named argument called compose
- **WHEN** a call passes a named argument called `compose` (`g(compose = 1)`) or a closure declares a parameter called `compose` (`{ compose -> compose }`)
- **THEN** `compose` is parsed as the argument name or parameter name, not as a modifier or an assignment

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
- **Borrowed struct fields are rejected in the parser.** Enforcing the zero-cost memory invariant at AST construction means no struct can ever carry a lifetime dependency.
- **`package` declaration versus `package` modifier.** A lookahead (`is_package_decl_ahead`) treats `package` followed by a dotted name as a declaration and `package fun` (or `struct`, `enum`, `val`, `var`) as visibility.
- **A fixed nesting limit (256) instead of a growable stack.** Recursive descent overflowed the stack on deep input and aborted the process. A fixed limit keeps the diagnostic the same for every build of the compiler and bounds the tree for every later recursive pass; growing the stack would need a dependency with a build script.
- **Fail-fast errors.** The parser stops at the first syntax error with an exact span and never panics. Multi-error recovery is left for LSP support.
