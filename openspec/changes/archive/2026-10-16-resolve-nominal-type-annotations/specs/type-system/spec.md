# Spec Delta

## MODIFIED Requirements

### Requirement: Function Signature and Invocation Typechecking
Function calls and closures MUST validate argument count, argument types, default parameter handling, and enforce composable calling context constraints across closure boundaries.

#### Scenario: Call argument type match
- **WHEN** calling `fun add(x: Int, y: Int): Int` with arguments `(1, 2)`
- **THEN** the call expression typechecks successfully with type `Int`

#### Scenario: Arity or argument type mismatch rejected
- **WHEN** calling a function with incorrect number of arguments or incompatible types
- **THEN** the type checker emits a call signature mismatch error (`E4004`)

#### Scenario: Default parameter omission succeeds
- **WHEN** calling a function omitting arguments that have default expressions
- **THEN** the call expression typechecks successfully

#### Scenario: Closure checking against expected function type
- **WHEN** a closure `{ x -> x + 1 }` is passed to a parameter expecting `(Int) -> Int`
- **THEN** the closure parameters and body typecheck against the expected function signature

#### Scenario: Calling composable function outside composable context rejected
- **WHEN** a standard function or non-composable closure attempts to invoke a function declared with the `compose` modifier (`compose fun`)
- **THEN** the type checker rejects the call with an invalid compose context error (`E4008`)

#### Scenario: Calling composable function within composable context succeeds
- **WHEN** a function or closure declared with `compose fun` invokes another `compose fun`
- **THEN** the call expression typechecks successfully

#### Scenario: Exported function missing return type rejected with dedicated code
- **WHEN** a `public` or `package` function declaration omits its return type annotation (e.g. `public fun greet(name: String) { }`)
- **THEN** the type checker emits a missing return type error (`E4011`) naming the function, and does NOT report it as a call signature mismatch (`E4004`)

### Requirement: User-Defined Struct and Nominal Type Validation
The type checker MUST validate struct instantiations, field access expressions, and enum variant constructors.

#### Scenario: Struct instantiation and field access
- **WHEN** a struct `Point { val x: Int, val y: Int }` is instantiated and `p.x` is accessed
- **THEN** instantiation verifies all required fields and `p.x` evaluates to `Int`

#### Scenario: Accessing unknown field rejected
- **WHEN** accessing a field not declared on the target struct
- **THEN** the type checker emits an unknown field error (`E4006`)

#### Scenario: Field access through an annotated struct type
- **WHEN** a value's type comes from an annotation naming a declared struct (a parameter `user: User`, a local `val p: Point = ...`, or a struct field `val user: User`) and a field of it is read
- **THEN** the field resolves against the declared struct and the access has the field's declared type, including chained access such as `order.user.name`

#### Scenario: Field access through a borrowed reference
- **WHEN** a parameter is declared `user: &User` or `user: &mut User` and `user.name` is read
- **THEN** the reference is peeled and `user.name` has the declared field type of `User`

#### Scenario: Field assignment through a mutable borrow
- **WHEN** a parameter is declared `user: &mut User` and the body assigns `user.name = n` to a `var` field with a compatible value
- **THEN** the assignment typechecks successfully, while assigning to a `val` field through the same borrow emits `E4003`

#### Scenario: Safe navigation on an annotated nullable struct
- **WHEN** a value is declared with type `User?` and `user?.name` is accessed
- **THEN** the access resolves against `User` and evaluates to the field type made nullable (`String?`)

#### Scenario: Field access on a struct imported from another file
- **WHEN** a file imports a module exporting `public struct User` and declares a parameter `user: User`
- **THEN** `user.name` resolves against the imported struct's ABI field list and typechecks with the exported field type

#### Scenario: Unknown nominal type name stays opaque
- **WHEN** an annotation names a type that is neither a primitive nor a declared or imported struct or enum
- **THEN** the annotation is accepted as an opaque nominal type, and accessing a member on a value of that type emits `E4006`
