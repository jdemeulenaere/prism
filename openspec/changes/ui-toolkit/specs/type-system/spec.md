# Type System Delta

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
- **WHEN** an annotation names a type that is neither a primitive nor a declared or imported struct or enum
- **THEN** the annotation is accepted as an opaque nominal type, and accessing a member on a value of that type emits `E4006`

## ADDED Requirements

### Requirement: A Struct May Have a Companion Value
A top-level `val` MAY have the name of a struct declared in the same file, as that struct's companion value. The name MUST then mean the struct in a type position and the struct's constructor as the callee of a call. Anywhere else in a value position, as a receiver or before `.field` included, it MUST mean the value.

#### Scenario: A companion value and its struct
- **WHEN** a file declares `struct Color : Copy { val argb: Int }`, `struct Colors : Copy { val Red: Color }`, `val Color: Colors = Colors(Red = Color(0xFFFF0000))` and `fun pick(c: Color): Int = c.argb`, and `fun main() { println(pick(Color.Red)); println(pick(Color(7))) }` runs
- **THEN** it prints `4294901760` and `7`: `Color` is the struct in `c: Color`, the constructor in `Color(7)`, and the value in `Color.Red`

#### Scenario: An extension called on a companion value
- **WHEN** a file declares `struct Gap : Copy { val size: Int }`, `struct Gaps : Copy { val None: Gap }`, `val Gap: Gaps = Gaps(Gap(0))` and `fun Gaps.of(size: Int): Gap = Gap(size)`, and `fun main() { println(Gap.of(8).size) }` runs
- **THEN** it prints `8`

### Requirement: Only a Same-File Struct and Value Share a Name
Every other pair of top-level declarations with one name MUST still be rejected with `E4010`: two `val`s, a `val` and an enum, function or extension, and a `val` and a struct that are not declared in the same file, an imported one included.

#### Scenario: A value named like an imported struct
- **WHEN** a file imports a module exporting `public struct User` and declares `val User: Int = 1`
- **THEN** the type checker emits `E4010` for `User`

#### Scenario: A value named like an enum
- **WHEN** a file declares `enum Mode { On, Off }` and `val Mode: Int = 1`
- **THEN** the type checker emits `E4010` for `Mode`

### Requirement: Importing a Struct With Its Companion Value
Importing a module MUST bring a public struct and its public companion value under their shared name, each meaning what it means in its own file.

#### Scenario: Using an imported companion value
- **WHEN** `lib/shape.prism` declares `public struct Size : Copy { val w: Int }`, `public struct Sizes : Copy { val Zero: Size }` and `public val Size: Sizes = Sizes(Size(0))`, and `app/main.prism` imports `/lib/shape` and declares `fun main() { val s: Size = Size.Zero; println(s.w + Size(2).w) }`
- **THEN** `prism run app/main.prism` prints `2`
