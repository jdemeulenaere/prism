# Syntax Specification Delta

## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: Assignment Is a Statement
An assignment, with `=` or a compound operator such as `+=`, MUST be a statement, never an expression. It MAY appear as a statement in a block or closure body, or as the unbraced body of an `if`/`else` branch, a `when` branch, or a `while` or `for` loop, where it forms a block with no tail. Anywhere else the parser MUST report `E2001`.

#### Scenario: Assignments as unbraced branch bodies
- **WHEN** parsing `if (c) x = 1 else x = 2` and `when (n) { 0 -> x = 3, else -> x = 4 }` inside a function body
- **THEN** each branch body is a block holding the assignment, with no tail

#### Scenario: Parenthesized assignment as a value rejected
- **WHEN** parsing `fun f() { var x = 0; val y = (x = 1) }` or `fun f() { var x = 0; g((x = 1)) }`
- **THEN** the parser reports `E2001`, while `g(x = 1)` still parses as a named argument
