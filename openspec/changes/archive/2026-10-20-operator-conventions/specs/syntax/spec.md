## ADDED Requirements

### Requirement: The operator Modifier
A function declaration MAY carry `operator` after its visibility and any `external` or `intrinsic` modifier, and before `compose` and the receiver mode (`public operator mut fun Bag.get(...)`). `operator` twice or in another position MUST be rejected with `E2001`. `operator` MUST be a contextual modifier only there, directly before `compose`, `mut`, `consuming` or `fun`, and a name elsewhere.

#### Scenario: Operator declarations parse
- **WHEN** parsing `operator fun Vector2.plus(other: Vector2): Vector2 = other`, `public operator mut fun Bag.get(index: Int): &mut Item = &mut items[index]`, `operator consuming fun Text.plus(other: &Text): Text = this` and `public intrinsic operator fun <T> List<T>.contains(element: &T): Bool`
- **THEN** each parses as a function marked `operator`, with its visibility, library modifier and receiver mode as written

#### Scenario: Operator modifiers in the wrong order rejected
- **WHEN** parsing `operator public fun P.plus(o: P): P = o`, `mut operator fun P.minus(o: P): P = o`, `operator operator fun P.times(o: P): P = o` and `operator intrinsic fun P.div(o: P): P`
- **THEN** the parser reports `E2001` for each

#### Scenario: operator as an identifier
- **WHEN** parsing `fun operator(operator: Int): Int = operator`, `fun f() { val operator = 1; g(operator = operator) }` and `x.operator()`
- **THEN** each `operator` parses as a function, parameter, variable, argument or member name, not as a modifier

### Requirement: Membership Operators
`a in b` and `a !in b` MUST parse as binary expressions, binding looser than `?:` and tighter than the comparison operators, and MUST be non-associative: an operand of a membership operator MUST NOT itself be an unparenthesized membership test (`E2001`). The lexer MUST read `!in` as one token when the character after `in` cannot continue an identifier. A newline before `in` or `!in` MUST end the expression.

#### Scenario: Membership precedence
- **WHEN** parsing `x in xs == true`, `a ?: b in xs`, `n in 0..<k + 1` and `!(x in xs)`
- **THEN** they parse as `((x in xs) == true)`, `((a ?: b) in xs)`, `(n in (0 ..< (k + 1)))` and the negation of `(x in xs)`

#### Scenario: Not-in tokens
- **WHEN** tokenizing `x !in xs`, `x!in xs` and `!inside`
- **THEN** the first two yield an identifier, one `!in` token and an identifier, and the third yields `!` and the identifier `inside`

#### Scenario: Chained membership rejected
- **WHEN** parsing `a in b in c` and `a !in b in c`
- **THEN** the parser reports `E2001` at the second operator, while `(a in b) in c` parses

#### Scenario: Loops and patterns keep their in
- **WHEN** parsing `for (x in xs) {}` and `when (n) { in 0..<10 -> 1; else -> 2 }`
- **THEN** each parses as before: a loop over `xs` and an `in` range pattern

### Requirement: Index Lists
An index expression MUST hold one or more comma-separated index expressions, `a[i]` or `a[i, j]`, with an optional trailing comma. `a[]` MUST be rejected with `E2001`. An index list is valid wherever an index expression is, as an assignment or compound assignment target included.

#### Scenario: Several indices
- **WHEN** parsing `m[1, 2]`, `m[r, c] = 0.0`, `m[i, i + 1] += 1.0` and `grid[0][1]`
- **THEN** the first three are index expressions of `m` with two indices, used as a value, an assignment target and a compound assignment target, and the last is an index of an index

#### Scenario: An empty index list rejected
- **WHEN** parsing `fun f(m: M): Int = m[]`
- **THEN** the parser reports `E2001` at `]`
