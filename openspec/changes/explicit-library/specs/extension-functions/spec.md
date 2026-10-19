## ADDED Requirements

### Requirement: Extensions Resolve Through the File and Its Imports
A call `value.name(...)` MUST resolve to an extension `name` on the value's type after `&`, `&mut` and `Heap<T>` layers, declared in the file or exported by a direct import, a library module's included (`standard-library`, "Library Names Are Imported"). The file's own match MUST shadow imported ones. Several imported matches MUST be `E4012` at use, naming their modules, and none `E4006`. Calling an imported extension MUST NOT need an import of its receiver type's module.

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

#### Scenario: Library extensions are imported
- **WHEN** a file declaring `import /prism/collections` declares `fun f(xs: &List<Int>): Bool = xs.any { x -> x > 2 }`, and the same declaration is checked in a file without imports
- **THEN** the first typechecks, the call resolving to the extension `any` of `/prism/collections`, and the second is rejected with `E4006` for `any`, with help naming `import /prism/collections`

## MODIFIED Requirements

### Requirement: Context Extensions Resolve Layer by Layer
A call MUST resolve in the first layer (the file's own extensions, then the imports') that declares an extension of its receiver and name, by the rule of "Extensions Differing Only by Context", and MUST be rejected with `E4034` when that layer has no match, naming the contexts it declares. Several imports exporting a matching extension of one receiver, name and context MUST be `E4012`.

#### Scenario: The file's own extension shadows an imported context extension
- **WHEN** `layout/scope.prism` exports `public struct Modifier : Copy { val fraction: Double }`, `public struct RowScope : Copy {}`, `public fun row(f: RowScope.() -> Unit): Unit { f(RowScope()) }` and `context(scope: RowScope) public fun Modifier.weight(w: Double): Modifier = Modifier(w)`, and a file importing `/layout/scope` declares `fun Modifier.weight(w: Double): Bool = true` and `fun f() { row { val b: Bool = Modifier(0.0).weight(1.0) } }`
- **THEN** the file typechecks successfully, the call resolving to the file's own extension

#### Scenario: Missing context names the declared contexts
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `struct ColumnScope : Copy {}`, `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` and `context(scope: ColumnScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` are declared and `fun f(): Modifier = Modifier(0.0).weight(1.0)` is checked
- **THEN** the type checker emits `E4034` at the call, saying that `weight` is declared for the contexts `RowScope` and `ColumnScope` and that this position has no receiver

#### Scenario: Ambiguous imported context extensions
- **WHEN** `ui/scope.prism` exports `public struct Modifier : Copy { val fraction: Double }`, `public struct RowScope : Copy {}`, `public struct ColumnScope : Copy {}` and `public fun row(f: RowScope.() -> Unit): Unit { f(RowScope()) }`, `a/lib` and `b/lib` each import `/ui/scope` and export `context(scope: RowScope) public fun Modifier.weight(w: Double): Modifier = Modifier(w)`, and a file importing `/ui/scope`, `/a/lib` and `/b/lib` declares `fun f() { row { val m = Modifier(0.0).weight(1.0) } }`
- **THEN** the type checker emits `E4012` naming `a/lib` and `b/lib`, and the file typechecks successfully when `b/lib`'s extension is declared for the context `ColumnScope` instead

## REMOVED Requirements

### Requirement: Extension Calls Resolve Through the File and Its Imports
**Reason**: Its scenario "Prelude extensions need no import" no longer holds: library extensions are imported like any other.
**Migration**: See "Extensions Resolve Through the File and Its Imports", which keeps every other scenario and replaces that one with "Library extensions are imported".
