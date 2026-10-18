# Syntax Specification Delta

## ADDED Requirements

### Requirement: Escaping Parameter Modifier
A parameter of a function declaration, or a parameter in a function type's parameter list, MAY be written with `escaping` before its type when that type is a function type, composable or not, or a parenthesized nullable function type. `escaping` before any other type, or in any other type position, MUST be rejected with `E2001`, saying that `escaping` marks function-typed parameters.

#### Scenario: Escaping parameters
- **WHEN** parsing `fun button(onClick: escaping () -> Unit, content: escaping compose () -> Unit) {}`, `fun later(f: escaping (() -> Unit)? = null) {}` and `struct Bus { val register: (escaping () -> Unit, Int) -> Unit }`
- **THEN** `onClick`, `content` and `f` are escaping parameters of types `() -> Unit`, `compose () -> Unit` and `(() -> Unit)?`, and the first parameter of the type of `register` is escaping while the second is not

#### Scenario: Escaping on another type rejected
- **WHEN** parsing `fun f(n: escaping Int) {}`, `fun g(x: escaping (Int)) {}`, `fun h() { val k: escaping () -> Unit = {} }` or `fun m(): escaping () -> Unit = {}`
- **THEN** the parser reports `E2001` at `escaping` for each, saying that `escaping` marks parameters of function type

## MODIFIED Requirements

### Requirement: Contextual Keyword Identifier Permissibility
Modifiers such as `compose`, `consuming` and `escaping` MUST function contextually, permitting their use as regular identifiers in member access and parameter declarations. `consuming` is a modifier only directly before `fun` at the start of a declaration. `escaping` is a modifier only at the start of a parameter's type and directly before `(` or `compose`; elsewhere it is a name, also a type name.

#### Scenario: Member access on compose identifier
- **WHEN** code accesses a property or method named compose (`pipeline.compose(step)`)
- **THEN** `compose` is parsed as a valid identifier expression rather than a modifier keyword

#### Scenario: Named argument called compose
- **WHEN** a call passes a named argument called `compose` (`g(compose = 1)`) or a closure declares a parameter called `compose` (`{ compose -> compose }`)
- **THEN** `compose` is parsed as the argument name or parameter name, not as a modifier or an assignment

#### Scenario: consuming as an identifier
- **WHEN** parsing `fun consuming(consuming: Int): Int = consuming`, `fun f() { val consuming = 1; g(consuming = consuming) }` and `x.consuming()`
- **THEN** each `consuming` parses as a function, parameter, variable, argument or member name, not as a modifier

#### Scenario: escaping as an identifier
- **WHEN** parsing `fun escaping(escaping: Int): Int = escaping`, `fun f(x: escaping, y: escaping?) {}`, `fun g() { val escaping = 1; h(escaping = escaping) }` and `x.escaping()`
- **THEN** each `escaping` parses as a function, parameter, variable, argument, member or type name, not as a modifier
