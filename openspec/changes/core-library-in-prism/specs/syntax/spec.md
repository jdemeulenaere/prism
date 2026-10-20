## ADDED Requirements

### Requirement: Extension Property Getters
A `val` declaration with a receiver and without `intrinsic` MUST declare an extension property with a getter: an optional visibility, `val`, an optional type parameter list, a receiver type, `.`, the name, `:` and its type, then `get`, `(`, `)` and either `=` with an expression or a block. The getter MAY follow the type on the same line or after newlines (`public val Int.dp: Dp get() = Dp(this)`). `get` MUST be a keyword only there, and a name everywhere else.

#### Scenario: Extension property getters parse
- **WHEN** parsing `public val Int.dp: Dp get() = Dp(this)`, `val <T> List<T>.second: &T` followed by a newline and `get() = &this[1]`, and `private val Box.area: Int get() { return w * h }`
- **THEN** each parses as an extension property with its visibility, type parameters, receiver type, name, type and getter body, the second with an expression body and the third with a block body

#### Scenario: get as an identifier
- **WHEN** parsing `fun get(get: Int): Int = get`, `val get = 1`, `fun f(b: Bag) { b.get(0); val x = get }` and `struct S { val get: Int }`
- **THEN** each `get` parses as a function, parameter, variable, member, field or callee name

### Requirement: Extension Property Declarations Are Restricted
An extension property without `intrinsic` MUST have a getter, MUST NOT have an initializer, and MUST NOT be `var`. `compose` or a receiver mode before it, a type parameter not occurring in its receiver type, and a `Heap<...>` or type-parameter receiver (as for functions) MUST be rejected. Each violation MUST be `E2001`. The message for an initializer suggests `get() =`, and the one for a missing getter says that an extension property has no stored value.

#### Scenario: Extension properties without a getter rejected
- **WHEN** parsing `val String.twice: Int` followed by a newline and `fun f() {}`, `val Int.half: Int = 2`, `var Int.twice: Int get() = 2`, `compose val Int.x: Int get() = 1`, and `val <T, R> List<T>.other: Int get() = 0`
- **THEN** the parser reports `E2001` for each, the second with help suggesting `get() = 2` and the last saying that `R` does not occur in the receiver type

#### Scenario: Property receivers follow extension receivers
- **WHEN** parsing `val Heap<User>.size: Int get() = 0` and `val <T> T.me: Int get() = 1`
- **THEN** the parser reports `E2001` for each, the first saying that reads look through `Heap`, so the property is declared on its contents, and the second that a bare type parameter cannot be a receiver

## MODIFIED Requirements

### Requirement: Intrinsic Extension Properties
`intrinsic val`, optionally after a visibility modifier, MUST declare an extension property: an optional type parameter list, a receiver type, `.`, the property name, `:` and its type (`public intrinsic val <T> List<T>.size: Int`), with no initializer and no getter. An intrinsic property with an initializer or a getter MUST be rejected with `E2001`. A `val` declaration with a receiver and without `intrinsic` is an extension property with a getter ("Extension Property Getters").

#### Scenario: Intrinsic properties parse
- **WHEN** parsing `public intrinsic val String.length: Int` and `public intrinsic val <T> List<T>.size: Int`
- **THEN** each parses as an extension property with its receiver type, name and type

#### Scenario: Extension properties outside the library form rejected
- **WHEN** parsing `val String.twice: Int = 2`, `intrinsic val String.a: Int = 1` or `intrinsic val String.b: Int get() = 1`
- **THEN** the parser reports `E2001`
