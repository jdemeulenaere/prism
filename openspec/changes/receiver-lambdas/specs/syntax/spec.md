# Syntax Specification Delta

## ADDED Requirements

### Requirement: Receivers of Function Types
The receiver of a function type MUST be a single type name, optionally with type arguments, and MAY be a type parameter in scope. A nullable, `&` or `&mut`, parenthesized, function or `Heap<...>` receiver MUST be rejected with `E2001`, with the message an extension receiver of that form gets, and `consuming` before a receiver MUST be rejected with `E2001`, saying that a function type's receiver is shared or `mut`.

#### Scenario: Receiver forms
- **WHEN** parsing `fun <T> with(receiver: &T, block: T.() -> Unit) { }`, `fun f(a: List<Int>.(Int) -> Bool) { }` and `fun g(h: (ColumnScope.() -> Unit)?) { }`
- **THEN** the receivers are the type parameter `T` and `List<Int>`, and `h` is a nullable function type whose receiver is `ColumnScope`

#### Scenario: Unsupported receivers rejected
- **WHEN** parsing `fun f(a: User?.() -> Unit) { }`, `fun g(a: &User.() -> Unit) { }`, `fun h(a: Heap<User>.() -> Unit) { }` or `fun k(a: consuming User.() -> Unit) { }`
- **THEN** the parser reports `E2001` for each

### Requirement: Context Parameter Declarations
A top-level function declaration MAY start with one context parameter, `context(name: Type)`, before its visibility and every other modifier. The type MUST follow the rules of an extension receiver and MUST NOT mention the declaration's own type parameters. A second context parameter, an empty `context()`, a context parameter without a name or type, and `context(...)` before a struct, enum or `val` declaration or before an `external` or `intrinsic` function MUST be rejected with `E2001`.

#### Scenario: Context parameter declarations
- **WHEN** parsing `context(scope: RowScope) public fun Modifier.weight(weight: Double, fill: Bool = true): Modifier = this`, `context(scope: ColumnScope) compose fun Divider() { }` and `context(s: RowScope) fun <T> Box<T>.cell(): Int = 1`
- **THEN** each parses as a function with the context parameter `scope` of type `RowScope`, `scope` of type `ColumnScope` or `s` of type `RowScope`, with its visibility, `compose` flag and receiver as written

#### Scenario: Invalid context parameters rejected
- **WHEN** parsing `context(a: RowScope, b: ColumnScope) fun f() { }`, `context() fun g() { }`, `context(RowScope) fun h() { }`, `context(s: RowScope) struct S { }`, `context(s: RowScope) val v = 1`, `context(s: RowScope) private external fun e(): Int`, `public context(s: RowScope) fun m() { }` or `context(s: Box<T>) fun <T> k() { }`
- **THEN** the parser reports `E2001` for each

## MODIFIED Requirements

### Requirement: Function Type Syntax
The type grammar MUST accept function types `(A, B) -> R`, including the zero-parameter form `() -> R`, optionally prefixed by `compose` to mark a composable function type. A function type MAY name a receiver before its parameter list, `T.(A, B) -> R`, and `mut` before the receiver marks it mutable, `mut T.(A) -> R`; `compose` comes before `mut`. `()` alone is the unit type, a single parenthesized type without an arrow is that type, and `->` is right-associative.

#### Scenario: Parameterless function type
- **WHEN** parsing `fun f(onClick: () -> Unit) { }`
- **THEN** the parameter type is a non-composable function type with no parameters returning `Unit`

#### Scenario: Composable function type
- **WHEN** parsing `fun Column(content: compose () -> Unit) { }` and `fun Row(item: compose (Int) -> Unit) { }`
- **THEN** each parameter type is a composable function type, and the exported ABI type string starts with `compose `

#### Scenario: Higher-order function type
- **WHEN** parsing `fun f(g: (Int) -> (Int) -> Int) { }`
- **THEN** the parameter type is a function from `Int` to a function from `Int` to `Int`

#### Scenario: Nullable function type needs parentheses
- **WHEN** parsing `fun f(a: (() -> Unit)?, b: () -> Unit?) { }`
- **THEN** `a` is a nullable function type, and `b` is a function type returning `Unit?`

#### Scenario: Function types with a receiver
- **WHEN** parsing `fun Column(content: compose ColumnScope.() -> Unit) { }`, `fun build(f: mut Builder.(String) -> Unit) { }`, `fun later(f: escaping compose mut Builder.() -> Unit) { }` and `fun g(h: Scope.() -> Scope.() -> Int) { }`
- **THEN** `content` is a composable function type with the shared receiver `ColumnScope` and no other parameters, `f` of `build` a function type with the mutable receiver `Builder` and one `String` parameter, `f` of `later` an escaping composable function type with the mutable receiver `Builder`, and `h` a function type with the receiver `Scope` returning a function type with the receiver `Scope`

### Requirement: Contextual Keyword Identifier Permissibility
`compose`, `consuming`, `context`, `escaping`, `external` and `intrinsic` MUST be contextual modifiers, usable as identifiers in member access and parameter declarations. `consuming` is one only directly before `fun` at a declaration's start; `context` only at a top-level declaration's start directly followed by `(`; `external` and `intrinsic` only at a top-level declaration's start, where "Library Declaration Modifiers" places them; `escaping` only at a parameter type's start, before `(`, `compose`, `mut` or a type name. Elsewhere each is a name, `escaping` also a type name.

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
