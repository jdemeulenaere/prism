# Formatting Specification Delta

## ADDED Requirements

### Requirement: Receivers and Context Parameters Are Printed
The formatter MUST print a function type's receiver as written, with no space around its `.` and one space after `compose`, `mut` and `escaping`, and MUST print a context parameter on its own line directly above the rest of its declaration, as `context(name: Type)`. Comments before a declaration MUST stay above its context line.

#### Scenario: Receiver types and contexts printed
- **WHEN** formatting `fun f(a: ColumnScope . ()->Unit, b: escaping  compose  mut Builder.(Int)->Unit) {}` and `// Weight.` followed by `context( scope : RowScope ) public fun Modifier.weight(w: Double): Modifier = this`
- **THEN** the results are `fun f(a: ColumnScope.() -> Unit, b: escaping compose mut Builder.(Int) -> Unit) {}`, and `// Weight.`, `context(scope: RowScope)` and `public fun Modifier.weight(w: Double): Modifier = this` on three lines
