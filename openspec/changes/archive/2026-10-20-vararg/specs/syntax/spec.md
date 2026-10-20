## ADDED Requirements

### Requirement: Vararg Parameter Modifier
A parameter of a `fun` declaration MAY be written with `vararg` before its name (`vararg elements: T`), the written type being the element type. A second `vararg` parameter in one declaration, a default value on one, `escaping` or `attached` in its type, and `vararg` on a struct's constructor field, a closure parameter, a context parameter or a parameter in a function type MUST each be rejected with `E2001`.

#### Scenario: Vararg parameters
- **WHEN** parsing `fun sum(vararg xs: Int): Int = 0`, `public fun <T> listOf(vararg elements: T): List<T> = elements`, `fun Builder.items(vararg names: String, separator: String = ", ") {}` and `compose fun Row(vararg labels: String) {}`
- **THEN** `xs`, `elements`, `names` and `labels` are vararg parameters with the element types `Int`, `T`, `String` and `String`, and `separator` is an ordinary parameter after the vararg

#### Scenario: Misplaced vararg rejected
- **WHEN** parsing `fun a(vararg x: Int, vararg y: Int) {}`, `fun b(vararg x: Int = 1) {}`, `fun c(vararg f: escaping () -> Unit) {}`, `struct S(vararg xs: Int)`, `fun d(f: (vararg Int) -> Unit) {}` or `fun e() { val g = { vararg x -> x } }`
- **THEN** the parser reports `E2001` for each, the first saying that a function has at most one vararg parameter and the second that a vararg's default is the empty list

### Requirement: Spread Arguments
An argument of a call MAY start with `*`, after its name when it is named, marking a spread argument whose expression follows (`sum(*numbers)`, `listOf(elements = *xs)`, `f(1, *a.items())`). Which spreads a call accepts is the rule of `vararg-parameters`. A `*` after an expression in an argument MUST stay multiplication.

#### Scenario: Spread arguments parse
- **WHEN** parsing `sum(*numbers)`, `listOf(elements = *xs)`, `f(1, *a.items())` and `f(a * b, a *b)`
- **THEN** the first three have one spread argument each, of the expressions `numbers`, `xs` and `a.items()`, and the last has two multiplications as arguments

## MODIFIED Requirements

### Requirement: Contextual Keyword Identifier Permissibility
`attached`, `capture`, `compose`, `consuming`, `context`, `escaping`, `external`, `intrinsic` and `vararg` MUST be contextual modifiers. `capture` and `consuming` are ones only directly before `fun`; `context`, `external` and `intrinsic` only at a top-level declaration's start, `context` directly before `(`; `attached` and `escaping` only at a parameter type's start; `vararg` only before a parameter's name. Elsewhere each is a name, `attached`, `escaping` and `vararg` also type names.

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

#### Scenario: vararg as an identifier
- **WHEN** parsing `fun vararg(vararg: Int): Int = vararg`, `fun f(vararg vararg: Int) {}`, `fun g(x: vararg) {}`, `fun h() { val vararg = 1; k(vararg = vararg) }` and `x.vararg()`
- **THEN** each `vararg` parses as a function, parameter, variable, argument, member or type name, except the first `vararg` of `f`, which is the modifier of the parameter `vararg`

### Requirement: Writing Through a Reference
An assignment target MAY start with one or more prefix `*` operators followed by an operand parsed at prefix precedence, so postfix operators bind tighter (`*r.x = 1` is `*(r.x) = 1`). A `*` that starts any other expression, except a spread argument ("Spread Arguments"), MUST be rejected with `E2001`, with a message saying that `*` is only allowed on the left of `=` and that reads through references are implicit. A newline before `*` MUST end the previous expression.

#### Scenario: Write-through statement
- **WHEN** parsing the statements `*r = 5`, `**rr = 0` and `*pick(&mut a, &mut b) = 1`
- **THEN** each parses as an assignment whose target is `r`, `rr` or the call, under one `*` per prefix operator written

#### Scenario: Postfix binds tighter than prefix star
- **WHEN** parsing `*r.x = 1`
- **THEN** the assignment target is `*` applied to `r.x`

#### Scenario: Prefix * is not a binary operator after a newline
- **WHEN** a statement `*r = 5` follows a line ending in an expression (`val a = b` then `*r = 5`)
- **THEN** it parses as a new statement, by the existing rule that a newline before a binary operator ends the expression

#### Scenario: Write-through in an unbraced branch or loop body
- **WHEN** parsing `if (c) *r = 1 else *r = 2`, the `when` branch `0 -> *r = 1` and `while (c) *r = 0`
- **THEN** each body is an assignment through `r`

#### Scenario: Prefix star outside an assignment target rejected
- **WHEN** a `*` starts an expression that is neither an assignment target nor an argument (`val x = *r`, `return *r`, `(*r).x = 1`, `g((*r))`, or the statement `*r` with no `=`)
- **THEN** the parser reports `E2001` at the `*`, saying that `*` writes through a reference only on the left of `=` and that reads through references are implicit

#### Scenario: Multiplication unchanged
- **WHEN** parsing `a * b` and `a *b` on one line
- **THEN** both parse as multiplication
