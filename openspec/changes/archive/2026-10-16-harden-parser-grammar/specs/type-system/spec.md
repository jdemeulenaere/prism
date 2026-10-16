# Spec Delta

## ADDED Requirements

### Requirement: Parameterless Closures Are Checked as Closures
A closure without parameters, including a trailing lambda or brace argument, MUST be typechecked as a function value, never as an eagerly evaluated block. When checked against an expected function type, its body MUST be checked in that type's composable context. When the expected type is not a function type, the closure MUST be rejected with `E4002`.

#### Scenario: Trailing lambda passed to a non-function parameter rejected
- **WHEN** checking `fun takesInt(n: Int) { }` and `fun f() { takesInt { 42 } }`
- **THEN** the type checker reports a type mismatch (`E4002`) between `Int` and the closure's function type

#### Scenario: Non-composable trailing lambda inside a compose fun
- **WHEN** checking `compose fun Child() { }`, `fun later(f: () -> Unit) { }` and `compose fun P() { later { Child() } }`
- **THEN** the type checker reports an invalid compose context error (`E4008`) for the call to `Child`

#### Scenario: Parameterless trailing lambda checked against its parameter type
- **WHEN** checking `fun run(body: () -> Int): Int = body()` and `fun f(): Int = run { 1 }`
- **THEN** the closure checks against `() -> Int` and the program typechecks without errors

### Requirement: Composable Function Types Keep Their Compose Flag
A function type written with the `compose` prefix MUST denote a composable function type wherever it is used, and one written without it MUST denote a non-composable one. A closure checked against a composable function type MUST be checked in a composable context, and composable and non-composable function types MUST NOT be assignable to each other.

#### Scenario: Composable content lambda may call composables
- **WHEN** checking `compose fun Item() { }`, `compose fun Column(content: compose () -> Unit) { content() }` and `compose fun Screen() { Column { Item() } }`
- **THEN** the program typechecks without errors

#### Scenario: Composable function type is not a plain function type
- **WHEN** checking `fun later(f: () -> Unit) { }` and `compose fun P(content: compose () -> Unit) { later(content) }`
- **THEN** the type checker reports a type mismatch (`E4002`) because the compose flags differ
