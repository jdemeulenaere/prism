## ADDED Requirements

### Requirement: Attached Parameter Modifier
A parameter of a function declaration, or a parameter in a function type's parameter list, MAY be written with `attached` before its type where `escaping` may be written. `attached` before any other type, in any other type position, or together with `escaping`, MUST be rejected with `E2001`, saying that `attached` marks function-typed parameters that the composition keeps.

#### Scenario: Attached parameters
- **WHEN** parsing `fun Button(onClick: attached () -> Unit, content: compose () -> Unit) {}`, `fun runApp(content: attached compose () -> Unit) {}`, `fun later(f: attached (() -> Unit)? = null) {}` and `fun register(r: (attached (Bool) -> Unit) -> Int) {}`
- **THEN** `onClick`, `content` of `runApp` and `f` are attached parameters of types `() -> Unit`, `compose () -> Unit` and `(() -> Unit)?`, and the parameter of the type of `r` is attached

#### Scenario: Attached on another type or with escaping rejected
- **WHEN** parsing `fun f(n: attached Int) {}`, `fun g() { val k: attached () -> Unit = {} }` or `fun h(f: escaping attached () -> Unit) {}`
- **THEN** the parser reports `E2001` at `attached` for each

### Requirement: Capture Function Modifier
`capture` MAY appear directly before `fun`, after `intrinsic`, in a declaration with a receiver, to mark the function that closures given to `attached` parameters use to capture a value of the receiver's type. `capture` without `intrinsic`, without a receiver, or together with `compose`, `mut` or `consuming`, MUST be rejected with `E2001`.

#### Scenario: A capture function
- **WHEN** parsing `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>`
- **THEN** it is an intrinsic extension function marked as a capture function, with type parameter `T`, receiver `MutableState<T>` and name `weak`

#### Scenario: Capture outside its form rejected
- **WHEN** parsing `capture fun Int.weak(): Int = 1`, `public intrinsic capture fun weak(): Int` or `public intrinsic capture mut fun Int.weak(): Int`
- **THEN** the parser reports `E2001` for each

## MODIFIED Requirements

### Requirement: Contextual Keyword Identifier Permissibility
`attached`, `capture`, `compose`, `consuming`, `context`, `escaping`, `external` and `intrinsic` MUST be contextual modifiers, usable as identifiers in member access and parameter declarations. `capture` and `consuming` are ones only directly before `fun`; `context`, `external` and `intrinsic` only at a top-level declaration's start, `context` directly before `(`; `attached` and `escaping` only at a parameter type's start. Elsewhere each is a name, `attached` and `escaping` also type names.

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

#### Scenario: external and intrinsic as identifiers
- **WHEN** parsing `fun external(intrinsic: Int): Int = intrinsic`, `fun f() { val external = 1; g(intrinsic = external) }` and `x.intrinsic()`
- **THEN** each `external` and `intrinsic` parses as a function, parameter, variable, argument or member name, not as a modifier

#### Scenario: context as an identifier
- **WHEN** parsing `fun context(context: Int): Int = context`, `fun f() { val context = 1; g(context = context); context(context) }`, `struct context { val x: Int }` and `x.context()`
- **THEN** each `context` parses as a function, parameter, variable, argument, callee, type or member name, not as a modifier

#### Scenario: attached and capture as identifiers
- **WHEN** parsing `fun attached(attached: Int): Int = attached`, `fun f(x: attached, capture: Int) {}`, `fun g() { val capture = 1; h(attached = capture) }` and `x.capture()`
- **THEN** each `attached` and `capture` parses as a function, parameter, variable, argument, member or type name, not as a modifier
