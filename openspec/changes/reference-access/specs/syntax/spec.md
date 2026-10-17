# Syntax Specification Delta

## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: Pratt Expression Precedence and Associativity
The parser MUST parse expressions according to Pratt operator precedence, correctly ordering assignment, Elvis, logical, comparison, range, arithmetic, prefix, postfix, and function call operators.

#### Scenario: Arithmetic and comparison precedence
- **WHEN** parsing expression `a + b * c > d`
- **THEN** operator precedence structures AST as `((a + (b * c)) > d)`

#### Scenario: Non-associative assignment
- **WHEN** code attempts chained assignment (`a = b = c`)
- **THEN** the parser rejects the expression with a compile error requiring separate statements

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
