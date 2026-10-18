# Extension Functions Delta

## MODIFIED Requirements

### Requirement: Extension Calls Resolve Through the File and Its Imports
A call `value.name(...)` MUST resolve to an extension named `name` whose receiver type is the value's type after `&`, `&mut` and `Heap<T>` layers are looked through, declared in the file, exported by a direct import, or, in a workspace file, exported by a prelude module (`standard-library`). A file's own matching extension MUST shadow imported ones, and an imported one MUST shadow a prelude one. Several imported matches MUST be rejected at use with `E4012` naming their modules, and no match with `E4006`. Calling an imported extension MUST NOT require importing its receiver type's module.

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

#### Scenario: Prelude extensions need no import
- **WHEN** a file without imports declares `fun f(xs: &List<Int>): Bool = xs.any { x -> x > 2 }`
- **THEN** the file typechecks, the call resolving to the extension `any` of `/prism/collections`

### Requirement: Members Take Precedence Over Extensions
A call `value.name(...)` MUST invoke, whatever extensions are visible, a field `name` of the receiver's struct, else the built-in `clone()` on a `Clone` receiver or `unbox()` on a receiver that is a `Heap<T>` after its references, else an intrinsic extension of the library named `name` on the receiver's type (`standard-library`, "Intrinsic Extensions Are Members"): `toString()` on an `Int`, `Double`, `Bool` or `Char` and the members of `List<T>`. An intrinsic property (a `String`'s `length`, a list's `size` and `indices`) counts as a field. An extension MUST be considered only when no such member exists.

#### Scenario: A field wins over an extension
- **WHEN** a file declares `struct Button { val onClick: () -> String }` and `fun Button.onClick(): Int = 1`, and `fun f(b: Button) { val s: String = b.onClick() }`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, because the call invokes the field

#### Scenario: A built-in toString wins over an extension
- **WHEN** a file declares `fun Int.toString(): String = "x"` and `fun f(n: Int): String = n.toString()`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, whose call is the intrinsic `toString()` of `/prism/core`

#### Scenario: The built-in unbox wins over an extension on the contents
- **WHEN** a file declares `struct Gift { val n: Int }`, `fun Gift.unbox(): Int = n`, `fun f(h: Heap<Gift>): Gift = h.unbox()` and `fun g(h: Heap<Gift>): Int { val r: &Gift = &h; return r.unbox() }`
- **THEN** the file typechecks: the call in `f` is the built-in `unbox()` of the box, and the call in `g` is the extension, on the borrowed contents
