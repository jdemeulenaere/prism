# Extension Functions Specification Delta

## MODIFIED Requirements

### Requirement: Members Take Precedence Over Extensions
A call `value.name(...)` MUST invoke a field named `name` of the receiver's struct when it has one, the built-in `clone()` when `name` is `clone` and the receiver's type is `Clone`, the built-in `unbox()` when `name` is `unbox` and the receiver, after its references, is a `Heap<T>`, and the built-in `toString()` when `name` is `toString` and the receiver's type is `Int`, `Double`, `Bool` or `Char`, whatever extensions are visible; the built-in `length` of a `String` counts as a field. An extension MUST be considered only when no such member exists.

#### Scenario: A field wins over an extension
- **WHEN** a file declares `struct Button { val onClick: () -> String }` and `fun Button.onClick(): Int = 1`, and `fun f(b: Button) { val s: String = b.onClick() }`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, because the call invokes the field

#### Scenario: A built-in toString wins over an extension
- **WHEN** a file declares `fun Int.toString(): String = "x"` and `fun f(n: Int): String = n.toString()`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, whose call is the built-in `toString()`

#### Scenario: The built-in unbox wins over an extension on the contents
- **WHEN** a file declares `struct Gift { val n: Int }`, `fun Gift.unbox(): Int = n`, `fun f(h: Heap<Gift>): Gift = h.unbox()` and `fun g(h: Heap<Gift>): Int { val r: &Gift = &h; return r.unbox() }`
- **THEN** the file typechecks: the call in `f` is the built-in `unbox()` of the box, and the call in `g` is the extension, on the borrowed contents
