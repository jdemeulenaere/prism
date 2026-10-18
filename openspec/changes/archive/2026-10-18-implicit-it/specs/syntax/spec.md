# Syntax Specification Delta

## ADDED Requirements

### Requirement: The Name it Is an Identifier
`it` MUST be an ordinary identifier, never a keyword: it MAY name a variable, parameter, closure parameter, function, field or argument, and a closure MAY declare a parameter named `it` in its header. The syntax tree MUST NOT add an implicit parameter to a closure; whether a closure has one is decided by the type checker.

#### Scenario: it as a declared name
- **WHEN** parsing `fun it(it: Int): Int = it`, `fun f() { val it = 1; g(it = it) }`, `x.it()` and `run { it -> it + 1 }`
- **THEN** each `it` parses as a function, parameter, variable, argument, member or closure parameter name, and the closure has the one declared parameter `it`

#### Scenario: A header-less closure has no parameters in the syntax tree
- **WHEN** parsing `xs.forEach { println(it) }`
- **THEN** the closure has no header and no parameters, and `it` in its body is an identifier expression

## MODIFIED Requirements

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
