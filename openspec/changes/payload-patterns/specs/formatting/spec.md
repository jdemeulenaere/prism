# Formatting Specification Delta

## ADDED Requirements

### Requirement: Patterns and Destructuring Are Printed
The formatter MUST print patterns as written, in the spacing of calls: constructor patterns like calls (`Shape.Rect(val w, _)`, `Point(y = 0)`), bindings as `val name` or `var name`, one space around `if` before a guard and around `as` in a destructuring entry, and destructuring declarations as `val (a, b as c) = value`. Pattern lists break like argument lists.

#### Scenario: Patterns, guards and destructuring
- **WHEN** formatting `fun f(s:Shape,u:User):Int{val(name as n,email)=u;return when(s){Shape.Rect(val w,_)if w>1->1;Point(y=0),_->2}}`
- **THEN** the result is `fun f(s: Shape, u: User): Int {\n    val (name as n, email) = u\n    return when (s) {\n        Shape.Rect(val w, _) if w > 1 -> 1\n        Point(y = 0), _ -> 2\n    }\n}\n`
