# Syntax Specification Delta

## ADDED Requirements

### Requirement: Extension Function Declarations
A function declaration MAY name a receiver type before its name, `fun T.name(...)`, which declares an extension function on `T`. The receiver MUST be a single type name. A receiver with type arguments, a `?`, a `&` or `&mut`, or a parenthesized or function type MUST be rejected with `E2001`, with a message saying what receivers may be.

#### Scenario: Extension declarations
- **WHEN** parsing `fun User.greet(): String = "Hi " + name`, `public fun Int.squared(): Int = this * this` and `public compose fun User.Avatar(): Unit { }`
- **THEN** each parses as a function named `greet`, `squared` or `Avatar` with the receiver type `User`, `Int` or `User` and the shared receiver mode

#### Scenario: Receiver with type arguments rejected
- **WHEN** parsing `fun Heap<User>.size(): Int = 0`
- **THEN** the parser reports `E2001`, saying that a receiver type with type arguments is not supported yet

#### Scenario: Nullable or borrowed receiver rejected
- **WHEN** parsing `fun User?.greet(): String = "x"`, `fun &User.greet(): String = "x"` or `fun &mut User.rename() { }`
- **THEN** the parser reports `E2001`; for `User?` the message says to declare the extension on `User` and call it with `?.`, and for `&` it says that the receiver mode (`fun`, `mut fun`, `consuming fun`) decides how the receiver is borrowed

#### Scenario: Qualified receiver rejected
- **WHEN** parsing `fun models.User.greet(): String = "x"`
- **THEN** the parser reports `E2001`, since type names are not qualified

### Requirement: Receiver Mode Modifiers
`mut` or `consuming` MAY appear directly before `fun` in a declaration that has a receiver, after any visibility and `compose` modifiers, to declare the receiver mode. On a declaration without a receiver, before a visibility or `compose` modifier, or twice, they MUST be rejected with `E2001`.

#### Scenario: Mutating and consuming extensions
- **WHEN** parsing `mut fun User.rename(n: String) { name = n }`, `public consuming fun User.toAccount(): Account = Account(this)` and `package compose mut fun Counter.Increment(): Unit { }`
- **THEN** the declarations parse with the receiver modes mutable, consuming and mutable

#### Scenario: Receiver modifier without a receiver rejected
- **WHEN** parsing `mut fun f() { }` or `consuming fun g(): Int = 1`
- **THEN** the parser reports `E2001`, saying that `mut` and `consuming` modify the receiver of an extension function (`mut fun T.name()`)

#### Scenario: Modifiers in the wrong order rejected
- **WHEN** parsing `mut public fun User.a() { }`, `consuming compose fun User.b() { }` or `mut consuming fun User.c() { }`
- **THEN** the parser reports `E2001`

### Requirement: The this Expression
`this` MUST parse as an expression atom wherever an expression may start, with postfix operators applying to it (`this.name`, `this.greet()`, `&this`). `this` MUST stay a reserved keyword that cannot name a variable, parameter, function, field or named argument.

#### Scenario: this in expressions
- **WHEN** parsing the function bodies `this * this`, `this.name`, `this.greet()` and `use(&this)`
- **THEN** each parses, with `this` as the operand, the target of the member access or call, or the operand of `&`

#### Scenario: this as a name rejected
- **WHEN** parsing `fun f() { val this = 1 }` or `fun g(this: Int) { }`
- **THEN** the parser reports `E2001`

## MODIFIED Requirements

### Requirement: Contextual Keyword Identifier Permissibility
Modifiers such as `compose` and `consuming` MUST function contextually, permitting their use as regular identifiers in member access and parameter declarations. `consuming` is a modifier only directly before `fun` at the start of a declaration.

#### Scenario: Member access on compose identifier
- **WHEN** code accesses a property or method named compose (`pipeline.compose(step)`)
- **THEN** `compose` is parsed as a valid identifier expression rather than a modifier keyword

#### Scenario: Named argument called compose
- **WHEN** a call passes a named argument called `compose` (`g(compose = 1)`) or a closure declares a parameter called `compose` (`{ compose -> compose }`)
- **THEN** `compose` is parsed as the argument name or parameter name, not as a modifier or an assignment

#### Scenario: consuming as an identifier
- **WHEN** parsing `fun consuming(consuming: Int): Int = consuming`, `fun f() { val consuming = 1; g(consuming = consuming) }` and `x.consuming()`
- **THEN** each `consuming` parses as a function, parameter, variable, argument or member name, not as a modifier
