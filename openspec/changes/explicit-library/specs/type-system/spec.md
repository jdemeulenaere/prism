## MODIFIED Requirements

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

#### Scenario: Same-named variants in two enums
- **WHEN** two enums visible in one file declare a variant with the same name (`enum A { None }` and `enum B { None }`)
- **THEN** both declarations are accepted, the qualified paths `A.None` and `B.None` resolve, a bare `None` resolves as a `when` pattern over either enum, and a bare `None` elsewhere emits `E4001`

#### Scenario: Type names are not values
- **WHEN** a struct without a companion value, or an enum, is named as a value (`fun f(): User = User`), a field is read from a struct name (`User.name`), or an enum name is called (`A()`)
- **THEN** the type checker emits `E4002`, `E4006` and `E4004` respectively, while a struct constructor call `User(...)` and an enum variant path `A.X` or `A.Y(1)` still typecheck

#### Scenario: Unknown nominal type name stays opaque
- **WHEN** an annotation names a type that is neither a primitive nor a declared or imported struct or enum, nor a public struct or enum of a library module
- **THEN** the annotation is accepted as an opaque nominal type, and accessing a member on a value of that type emits `E4006`; an annotation naming a library module's type without its import is `E4001` (`standard-library`, "Library Names Are Imported")
