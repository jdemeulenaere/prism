# Extension Functions Specification Delta

## Purpose

The extension functions specification defines methods in Prism, which are extension functions only: how they are declared on any type in any file, how a call `value.name(...)` resolves statically through the file's own declarations and its imports, how names resolve inside an extension body, which extension declarations are rejected, and how calls on nullable receivers work. How the receiver is borrowed or moved at the call site is in the `references` and `ownership` specs.

## ADDED Requirements

### Requirement: Extension Functions Are the Only Methods
A method MUST be declared as an extension function `fun T.name(...)` on a struct, enum or built-in type, in any file, with the usual visibility modifiers, and MUST be called as `value.name(...)` with the receiver's static type choosing the extension. Its parameters, defaults, named arguments and `compose` flag MUST be checked as for a function. An extension is not a value: reading `value.name` without calling it MUST be rejected with `E4006`.

#### Scenario: Extension on a user type
- **WHEN** `struct User { val name: String }` and `public fun User.greet(): String = "Hi " + name` are declared and `fun f(u: User): String = u.greet()` is checked
- **THEN** the call resolves statically to the extension and has type `String`, and `name` in its body reads `this.name`

#### Scenario: Extension on a built-in type
- **WHEN** a file declares `fun Int.squared(): Int = this * this` and `fun Range.size(): Int = end - start`, and `fun f(): Int = 3.squared() + (0..<5).size()` is checked
- **THEN** the file typechecks successfully, and each call has type `Int`

#### Scenario: Extension on an enum
- **WHEN** `enum Color { Red, Green }` and `fun Color.next(): Color = Color.Green` are declared and `fun f(c: Color): Color = c.next()` is checked
- **THEN** the file typechecks successfully

#### Scenario: Same name on different receivers
- **WHEN** a file declares `struct User { val id: Int }`, `struct Team { val id: Int }`, `fun User.describe(): String = "user"`, `fun Team.describe(): Int = 1` and `fun describe(): Bool = true`, and `fun f(u: User, t: Team) { val a: String = u.describe(); val b: Int = t.describe(); val c: Bool = describe() }`
- **THEN** the file typechecks successfully, each call resolving to its own declaration

#### Scenario: Extension arguments are checked like a function's
- **WHEN** `fun User.greet(greeting: String = "Hi", mark: String = "!"): String` is declared and a body of `fun f(u: User)` runs `u.greet()`, `u.greet(mark = "?")` and `u.greet(1)`
- **THEN** the type checker accepts the first two calls and emits `E4004` for `u.greet(1)`

#### Scenario: Composable extension called outside a composable
- **WHEN** `compose fun User.Avatar()` is declared and `fun f(u: User) { u.Avatar() }` is checked
- **THEN** the type checker emits `E4008`

#### Scenario: An extension is not a value
- **WHEN** `fun User.greet(): String` is declared and `fun f(u: User) { val g = u.greet }` is checked
- **THEN** the type checker emits `E4006`, with help suggesting the call `u.greet()`

### Requirement: Extension Calls Resolve Through the File and Its Imports
A call `value.name(...)` MUST resolve to an extension named `name` whose receiver type is the value's type after `&`, `&mut` and `Heap<T>` layers are looked through, declared in the file or exported by a direct import. A file's own matching extension MUST shadow imported ones. Several imported matches MUST be rejected at use with `E4012` naming their modules, and no match with `E4006`. Calling an imported extension MUST NOT require importing its receiver type's module.

#### Scenario: Extension declared in another package
- **WHEN** `models` exports `public struct User { val name: String }`, package `ui` imports `/models` and declares `public fun User.shout(): String`, and `app` imports `/models` and `/ui` and calls `u.shout()` for `u: User`
- **THEN** the declaration and the call typecheck successfully, while the same call in a file that imports only `/models` reports `E4006`

#### Scenario: Calling an imported extension without importing the receiver's module
- **WHEN** `ui` also exports `public fun first(): User`, and a file that imports only `/ui` declares `fun f(): String = first().shout()`
- **THEN** the file typechecks successfully, without `E4014`, while reading `first().name` in that file reports `E4014`

#### Scenario: Ambiguous imported extension
- **WHEN** `a/lib` and `b/lib` each import `/models` and export `public fun User.tag(): String`, and a file importing `/models`, `/a/lib` and `/b/lib` calls `u.tag()`
- **THEN** the type checker emits `E4012` naming `a/lib` and `b/lib`, and the file typechecks successfully when `tag` is never called or when `b/lib`'s extension is declared on another receiver type instead

#### Scenario: Own extension shadows an imported one
- **WHEN** a file imports `/models` and `/a/lib`, declares its own `fun User.tag(): Int`, and runs `val t: Int = u.tag()`
- **THEN** the file typechecks successfully, the call resolving to the file's own extension

#### Scenario: Package extension across packages rejected
- **WHEN** `ui` declares `package fun User.internal(): Int` and a file in another package that imports `/ui` calls `u.internal()`
- **THEN** the type checker emits `E4007`

#### Scenario: Private extension invisible to importers
- **WHEN** `ui` declares `fun User.secret(): Int` without a visibility modifier and a file that imports `/ui` calls `u.secret()`
- **THEN** the type checker emits `E4006`

#### Scenario: Unknown method
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: User) { u.fly() }` is checked
- **THEN** the type checker emits `E4006` naming `fly` and `User`

### Requirement: Members Take Precedence Over Extensions
A call `value.name(...)` MUST invoke a field named `name` of the receiver's struct when it has one, and the built-in `clone()` when `name` is `clone` and the receiver's type is `Clone`, whatever extensions are visible. An extension MUST be considered only when no such member exists.

#### Scenario: A field wins over an imported extension
- **WHEN** `models` exports `public struct Button { val onClick: () -> String }`, `ui` exports `public fun Button.onClick(): Int`, and a file importing both runs `val s: String = b.onClick()` for `b: Button`
- **THEN** the file typechecks successfully, because the call invokes the field

### Requirement: The Receiver in Extension Bodies
Inside an extension body, `this` MUST be the receiver, a `val` binding of type `&T` in a `fun`, `&mut T` in a `mut fun` and `T` in a `consuming fun`, in scope in the body and in parameter defaults. A bare name that is not a local or parameter MUST resolve to `this.name` when the receiver's struct has that field, and, when called, to `this.name(...)` when an extension on the receiver type matches, before top-level declarations and imports. `this` elsewhere MUST be `E4001`.

#### Scenario: The type of this in each mode
- **WHEN** `struct User { var name: String }` is declared, and `fun User.me(): &User = this`, `mut fun User.reset() { this.name = "" }`, `fun User.bad() { this.name = "" }` and `consuming fun User.take(): User = this` are checked
- **THEN** the type checker accepts `me`, `reset` and `take`, and emits `E4003` for `bad`, because `this` is a shared borrow there

#### Scenario: Fields through implicit this
- **WHEN** `struct User { var name: String }` is declared, and `mut fun User.rename(n: String) { name = n }`, `fun User.label(name: String): String = name + this.name` and `fun User.clear() { name = "" }` are checked
- **THEN** the type checker accepts `rename` (assigning `this.name`) and `label` (where the parameter `name` shadows the field), and emits `E4003` for `clear`

#### Scenario: A receiver field shadows a top-level declaration
- **WHEN** a file declares `val id = "global"`, `struct User { val id: Int }` and `fun User.key(): Int = id`
- **THEN** the file typechecks successfully, because `id` in the body is the field

#### Scenario: Calling another extension on the implicit receiver
- **WHEN** `fun User.greet(): String = "Hi"` and `fun User.shout(): String = greet() + "!"` are declared
- **THEN** the file typechecks successfully, and `greet()` calls `this.greet()`

#### Scenario: A default value may read the receiver
- **WHEN** `struct User { val name: String }` and `fun User.greet(to: &String = name): String = "Hi " + to` are declared and `fun f(u: User): String = u.greet()` is checked
- **THEN** the file typechecks successfully

#### Scenario: this outside an extension
- **WHEN** `fun f() { val t = this }` is checked
- **THEN** the type checker emits `E4001`, saying that `this` is only available in an extension function

### Requirement: Extension Declarations Are Checked
Two extensions in one file with the same receiver type and name MUST be rejected with `E4010`, as MUST an extension named like a field of its receiver struct and an extension named `clone` on a `Clone` type, which could never be called. Extensions on different receivers, and an extension and a top-level function, MAY share a name. The receiver type MUST follow the rules of type annotations (`E4007`, `E4012`, `E4014`).

#### Scenario: Duplicate extension rejected
- **WHEN** a file declares `fun User.greet(): String = "a"` and `fun User.greet(x: Int): String = "b"`
- **THEN** the type checker emits `E4010` for the second declaration

#### Scenario: Extension shadowed by a field rejected
- **WHEN** a file declares `struct User { val name: String }` and `fun User.name(): String = "x"`
- **THEN** the type checker emits `E4010`, saying that the field `name` always takes precedence

#### Scenario: Extension named clone on a Clone type rejected
- **WHEN** a file declares `struct User : Clone { val name: String }`, `fun User.clone(): User = User("x")` and `fun String.clone(): String = "x"`
- **THEN** the type checker emits `E4010` for each, saying that the built-in `clone()` always takes precedence

#### Scenario: Receiver type from an unimported module
- **WHEN** `shop/api` exports `public fun first(): Order`, where `Order` is declared in `shop/models`, and a file that imports only `/shop/api` declares `fun Order.total(): Int = 0`
- **THEN** the type checker emits `E4014` at the receiver type, naming `import /shop/models`

#### Scenario: Use before an inferred extension declaration
- **WHEN** `fun g(u: User): String { val s: String = u.size(); return s }` precedes `fun User.size() = 42`
- **THEN** the type checker emits `E4002`, exactly as when the declarations are swapped

### Requirement: Safe-Navigation Calls
`value?.name(...)` MUST call the extension on the receiver when it is not `null` and give `null` otherwise, with the extension's return type made nullable (never nested). A chain of `?.` field accesses over a place (`t?.lead`) MUST be accepted as such a receiver. A `.` call on a nullable receiver MUST be rejected with `E4006`, with help suggesting `?.`. Extensions MUST NOT be declarable on nullable receiver types.

#### Scenario: Safe call on a nullable receiver
- **WHEN** `fun User.greet(): String` and `fun User.age(): Int?` are declared, and `fun f(u: User?): String? = u?.greet()` and `fun g(u: User?): Int? = u?.age()` are checked
- **THEN** both typecheck successfully: `u?.age()` has type `Int?`, not `Int??`

#### Scenario: Plain call on a nullable receiver rejected
- **WHEN** `fun User.greet(): String` is declared and `fun f(u: User?): String = u.greet()` is checked
- **THEN** the type checker emits `E4006`, with help suggesting `u?.greet()`

#### Scenario: Safe call chain over fields
- **WHEN** `struct User { val name: String }`, `struct Team { val lead: User? }` and `fun User.greet(): String` are declared, and `fun f(t: &Team?): String? = t?.lead?.greet()` is checked
- **THEN** the file typechecks successfully, and `t?.lead` is borrowed where it lives rather than reported as moved
