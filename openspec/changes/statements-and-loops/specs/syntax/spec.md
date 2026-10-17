# Spec Delta

## ADDED Requirements

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
An assignment MUST be a statement, never an expression. It MAY appear as a statement in a block or closure body, or as the unbraced body of an `if`/`else` branch, a `when` branch, or a `while` or `for` loop, where it forms a block with no tail. Anywhere else the parser MUST report `E2001`.

#### Scenario: Assignments as unbraced branch bodies
- **WHEN** parsing `if (c) x = 1 else x = 2` and `when (n) { 0 -> x = 3, else -> x = 4 }` inside a function body
- **THEN** each branch body is a block holding the assignment, with no tail

#### Scenario: Parenthesized assignment as a value rejected
- **WHEN** parsing `fun f() { var x = 0; val y = (x = 1) }` or `fun f() { var x = 0; g((x = 1)) }`
- **THEN** the parser reports `E2001`, while `g(x = 1)` still parses as a named argument

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

## MODIFIED Requirements

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
