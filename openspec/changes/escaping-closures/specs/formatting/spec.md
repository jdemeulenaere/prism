# Formatting Specification Delta

## ADDED Requirements

### Requirement: Escaping Modifiers Are Kept
The formatter MUST print `escaping` before each parameter type that has it, in declarations and in function types, followed by one space, and keep `compose` after it.

#### Scenario: Escaping parameters formatted
- **WHEN** formatting `fun b(o:escaping ()->Unit,f:escaping (()->Unit)?=null){}` and `fun c(content:escaping compose ()->Unit,r:(escaping ()->Unit)->Unit){}`
- **THEN** the results are `fun b(o: escaping () -> Unit, f: escaping (() -> Unit)? = null) {}` and `fun c(content: escaping compose () -> Unit, r: (escaping () -> Unit) -> Unit) {}`
