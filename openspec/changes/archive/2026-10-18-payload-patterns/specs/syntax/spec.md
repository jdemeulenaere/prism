# Syntax Specification Delta

## MODIFIED Requirements

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
