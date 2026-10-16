# Spec Delta

## ADDED Requirements

### Requirement: Braces Are Closures in Expression Position
A `{` in expression position MUST start a closure, with or without a parameter header. A `{` MUST start a block only in block position: a function body, an `if`/`else` branch body, a `when` branch body, or a statement in a block or closure body. A closure without a `->` header MUST have zero parameters, and every closure body MUST be a block.

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

### Requirement: Function Type Syntax
The type grammar MUST accept function types `(A, B) -> R`, including the zero-parameter form `() -> R`, optionally prefixed by `compose` to mark a composable function type. `()` alone is the unit type, a single parenthesized type without an arrow is that type, and `->` is right-associative.

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

### Requirement: Declarations in Statement Position Only
A local `val` or `var` declaration MUST appear only as a statement directly inside a block or closure body, and the parser MUST reject it anywhere else with `E2001`. That includes call arguments, operands, assigned values, conditions, returned values, expression function bodies, and unbraced `if`/`else` or `when` branch bodies. A declaration's initializer MUST NOT be an assignment.

#### Scenario: Declaration as a call argument rejected
- **WHEN** parsing `fun f() { g(val z = 1) }`
- **THEN** the parser reports `E2001`, stating that a declaration is only allowed as a statement

#### Scenario: Declaration as an assigned value rejected
- **WHEN** parsing `fun f() { x = val y = 3 }`
- **THEN** the parser reports `E2001`

#### Scenario: Declaration as an unbraced branch body rejected
- **WHEN** parsing `fun f(c: Bool): Int {\nif (c) val x = 1\nreturn x\n}`
- **THEN** the parser reports `E2001` at `val`, so `x` cannot leak out of the conditional

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
