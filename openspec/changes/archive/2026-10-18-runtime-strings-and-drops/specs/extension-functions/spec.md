# Extension Functions Specification Delta

## MODIFIED Requirements

### Requirement: Members Take Precedence Over Extensions
A call `value.name(...)` MUST invoke a field named `name` of the receiver's struct when it has one, the built-in `clone()` when `name` is `clone` and the receiver's type is `Clone`, and the built-in `toString()` when `name` is `toString` and the receiver's type is `Int`, `Double`, `Bool` or `Char`, whatever extensions are visible; the built-in `length` of a `String` counts as a field. An extension MUST be considered only when no such member exists.

#### Scenario: A field wins over an extension
- **WHEN** a file declares `struct Button { val onClick: () -> String }` and `fun Button.onClick(): Int = 1`, and `fun f(b: Button) { val s: String = b.onClick() }`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, because the call invokes the field

#### Scenario: A built-in toString wins over an extension
- **WHEN** a file declares `fun Int.toString(): String = "x"` and `fun f(n: Int): String = n.toString()`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, whose call is the built-in `toString()`

### Requirement: Extension Declarations Are Checked
Two extensions in one file with the same receiver type constructor (`Box` for `Box<Int>` and `Box<T>`) and name MUST be rejected with `E4010`, as MUST an extension that could never be called: one named like a field of its receiver struct, named `clone` on a `Clone` type, named `toString` on `Int`, `Double`, `Bool` or `Char`, or named `length` on `String`. Extensions on different receivers, and an extension and a top-level function, MAY share a name. The receiver type MUST follow the rules of type annotations (`E4007`, `E4012`, `E4014`).

#### Scenario: Duplicate extension rejected
- **WHEN** a file declares `fun User.greet(): String = "a"` and `fun User.greet(x: Int): String = "b"`
- **THEN** the type checker emits `E4010` for the second declaration

#### Scenario: Extension shadowed by a field rejected
- **WHEN** a file declares `struct User { val name: String }` and `fun User.name(): String = "x"`
- **THEN** the type checker emits `E4010`, saying that the field `name` always takes precedence

#### Scenario: Extension named clone on a Clone type rejected
- **WHEN** a file declares `struct User : Clone { val name: String }`, `fun User.clone(): User = User("x")` and `fun String.clone(): String = "x"`
- **THEN** the type checker emits `E4010` for each, saying that the built-in `clone()` always takes precedence

#### Scenario: Extensions named like built-in members rejected
- **WHEN** a file declares `fun Int.toString(): String = "x"`, `fun Char.toString(): String = "c"` and `fun String.length(): Int = 0`
- **THEN** the type checker emits `E4010` for each, saying that the built-in member always takes precedence

#### Scenario: Receiver type from an unimported module
- **WHEN** `shop/api` exports `public fun first(): Order`, where `Order` is declared in `shop/models`, and a file that imports only `/shop/api` declares `fun Order.total(): Int = 0`
- **THEN** the type checker emits `E4014` at the receiver type, naming `import /shop/models`

#### Scenario: Use before an inferred extension declaration
- **WHEN** `fun g(u: User): String { val s: String = u.size(); return s }` precedes `fun User.size() = 42`
- **THEN** the type checker emits `E4002`, exactly as when the declarations are swapped
