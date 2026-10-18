# Type System Specification Delta

## ADDED Requirements

### Requirement: String Templates Have Type String
A string template MUST have type `String`, and each of its parts MUST be checked as an expression with no expected type, in the scope where the template appears: a name part resolves as the identifier would (a local, a parameter, a top-level value, a field of the receiver through implicit `this` in an extension body, or `this`).

#### Scenario: Templates are strings
- **WHEN** `fun f(n: Int): String = "n = $n"` and `fun g(n: Int): Int = "n = $n"` are checked
- **THEN** the type checker accepts `f` and emits `E4005` for `g`, which returns a `String` where an `Int` is declared

#### Scenario: Names resolve as in code
- **WHEN** `struct User { val name: String }`, `fun User.toString(): String = "user $name"` and `fun User.label(prefix: String): String = "$prefix: $this (${name.length})"` are checked
- **THEN** the file typechecks successfully: `$name` reads the receiver's field, and `$this` inserts the receiver through its `toString()` extension

### Requirement: Built-In Text Forms of Template Parts
A template part MUST be read through any `&`, `&mut` and `Heap<T>` layers. A `String` part MUST be inserted as it is, and an `Int`, `Double`, `Bool` or `Char` part as the text its built-in `toString()` gives. A part of type `Nothing` MUST be accepted. A template MUST NOT consume or mutate any of its parts.

#### Scenario: Parts of every built-in text type
- **WHEN** `fun f(n: Int, d: &Double, b: Bool, c: Char, s: &String, t: String, h: Heap<Int>): String = "$n $d $b $c $s $t $h ${s.length} ${n > 0}"` is checked
- **THEN** the file typechecks successfully

#### Scenario: A part is not moved
- **WHEN** `fun f(s: String): String { val t = "$s!"; return s + t }` is checked
- **THEN** the file typechecks successfully, since the template does not move `s`

### Requirement: Other Template Parts Call toString
A part of any other non-nullable type MUST be inserted as the result of `toString()` on it, resolved exactly as the method call `part.toString()` would be: a field, then an extension of the file or of a direct import, with the precedence and import rules of method calls. That member MUST be callable without arguments and return `String`, and an extension MUST take a shared receiver: a `mut fun` or `consuming fun` `toString()` is never called implicitly.

#### Scenario: A toString extension gives a text form
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `enum Color { Red, Green }`, `fun Point.toString(): String = "($x, $y)"`, `fun Color.toString(): String = when (this) { Color.Red -> "red"; Color.Green -> "green" }` and `fun f(p: &Point, c: Color): String = "$p is $c"` are checked
- **THEN** the file typechecks successfully
- **WHEN** `lib/geo.prism` declares `public struct Point(val x: Int, val y: Int)` and `public fun Point.toString(): String = "($x, $y)"`, and a file importing `/lib/geo` declares `fun f(p: Point): String = "at $p"`
- **THEN** the importing file typechecks successfully, calling the imported extension

### Requirement: Parts Without a Text Form Are Rejected
A template part MUST be rejected with `E4026` at the part (the name of a name part, the expression of an expression part) when it is nullable (any `T?`, including `String?` and `null`), `Unit`, a function value or a type parameter, or when it has no `toString()` member usable by "Other Template Parts Call toString". The diagnostic MUST name the type and the reason.

#### Scenario: A nullable part rejected
- **WHEN** `fun f(name: String?, n: Int?): String = "Hello $name, ${n}"` is checked
- **THEN** the type checker emits `E4026` at `name` and at `n`, and accepts `fun g(name: String?): String = "Hello ${name ?: "guest"}"`

#### Scenario: Unit and function parts rejected
- **WHEN** `fun log() {}`, `fun f(): String = "${log()}"`, `fun g(h: () -> Int): String = "$h"` and `fun <T> k(t: T): String = "$t"` are checked
- **THEN** the type checker emits `E4026` in each of `f`, `g` and `k`

### Requirement: Text Form Diagnostics Name the Fix
An `E4026` diagnostic MUST carry help naming a fix: for a nullable part, a fallback with `?:` written with the part's text (`${n ?: ...}`); for a struct or enum without `toString()`, the extension to declare (`fun User.toString(): String`) and, when a name part is followed by text that starts with `.` and an identifier, the braced member (`${u.name}`); for a `mut fun` or `consuming fun` `toString()`, the explicit call (`${c.toString()}`).

#### Scenario: A struct without toString rejected
- **WHEN** `struct User { val name: String }` and `fun f(u: User): String = "user $u.name"` are checked
- **THEN** the type checker emits `E4026` at `u`, saying that `User` has no text form, with help suggesting `fun User.toString(): String` and `${u.name}`

#### Scenario: An unusable toString rejected
- **WHEN** `struct Point : Copy { val x: Int }`, `struct Counter { var n: Int }`, `fun Point.toString(): Int = x`, `mut fun Counter.toString(): String { n += 1; return "c" }`, `fun f(p: Point): String = "$p"` and `fun g(c: Counter): String = "$c"` are checked
- **THEN** the type checker emits `E4026` in `f`, saying that `toString()` of `Point` returns `Int`, and `E4026` in `g`, with help suggesting `${c.toString()}`

#### Scenario: Nullable help gives a fallback
- **WHEN** `fun f(n: Int?): String = "n = $n"` is checked
- **THEN** the type checker emits `E4026` at `n`, saying that a nullable `Int?` has no text form, with help suggesting `${n ?: ...}`
