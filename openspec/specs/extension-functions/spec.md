# Extension Functions Specification

## Purpose
The extension functions specification defines methods in Prism, which are extension functions only: how they are declared on any type in any file, how a call `value.name(...)` resolves statically through the file's own declarations and its imports, how names resolve inside an extension body, which extension declarations are rejected, and how calls on nullable receivers work. How the receiver is borrowed or moved at the call site is in the `references` and `ownership` specs.

## Requirements

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

### Requirement: Members Take Precedence Over Extensions
A call `value.name(...)` MUST invoke a field `name` of the receiver's struct. Failing that, the built-in `clone()` of a `Clone` receiver or `unbox()` of a `Heap<T>` behind its references. Failing that, a member `name` of its type (`standard-library`, "Intrinsic Extensions Are Members"), such as a scalar's `toString()`, whatever extensions are visible; a member property (`length`, `size`, `indices`) counts as a field. Then a receiver function value `name`, and only then an extension.

#### Scenario: A field wins over an extension
- **WHEN** a file declares `struct Button { val onClick: () -> String }` and `fun Button.onClick(): Int = 1`, and `fun f(b: Button) { val s: String = b.onClick() }`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, because the call invokes the field

#### Scenario: A built-in toString wins over an extension
- **WHEN** a file declares `fun Int.toString(): String = "x"` and `fun f(n: Int): String = n.toString()`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, whose call is the member `toString()` of `/prism/core`, `prism/core::Int.toString`

#### Scenario: The built-in unbox wins over an extension on the contents
- **WHEN** a file declares `struct Gift { val n: Int }`, `fun Gift.unbox(): Int = n`, `fun f(h: Heap<Gift>): Gift = h.unbox()` and `fun g(h: Heap<Gift>): Int { val r: &Gift = &h; return r.unbox() }`
- **THEN** the file typechecks: the call in `f` is the built-in `unbox()` of the box, and the call in `g` is the extension, on the borrowed contents

### Requirement: Only Visible Fields Count
Wherever a field takes part in resolving a name (a call `value.name(...)`, a property read, a bare name in an extension body), only a field visible in that file MUST count. An extension function or extension property named like a field of its receiver struct MUST be `E4010` only when that field is visible in the extension's file. A hidden field MUST NOT shadow a member, extension or outer binding of its name.

#### Scenario: An extension named like a hidden field
- **WHEN** `bank/account.prism` exports `public struct Account { private val id: Int, val owner: String }` and `public fun open(id: Int): Account = Account(id, "ann")`, and a file importing `/bank/account` and `/prism/core` declares `fun Account.id(): Int = 42` and `fun main() { println(open(7).id()) }`, and runs with `--check-allocations`
- **THEN** the file typechecks and stdout is `42`: the private field `id` is not visible there, so the call resolves to the file's own extension

#### Scenario: An extension named like a visible field is still rejected
- **WHEN** a file declares `struct Account { private val id: Int }` and `fun Account.id(): Int = 1`
- **THEN** the type checker emits `E4010` for the extension, saying that the field `id` always takes precedence in this file

#### Scenario: A hidden field does not shadow an outer name
- **WHEN** `lib/box.prism` exports `public struct Box : Copy { private val size: Int }` and `public fun box(): Box = Box(3)`, and a file importing `/lib/box` declares `val size: Int = 10` and `fun Box.twice(): Int = size * 2`
- **THEN** the file typechecks, and `size` in `twice` reads the file's top-level value

### Requirement: Receiver Function Values Come Before Extensions
A call `value.name(...)` that finds no member MUST, when the innermost local or parameter named `name` has a function type with a receiver whose receiver type is `value`'s type after its `&`, `&mut` and `Heap<T>` layers, call that value with `value` as its receiver (`type-system`, "Calling Function Values With a Receiver"). A local of any other type MUST be skipped, so extensions are found as before.

#### Scenario: A receiver function value wins over an extension
- **WHEN** a file declares `struct Scope : Copy {}`, `fun Scope.render(): Bool = true`, `fun f(s: Scope, render: Scope.() -> Int): Int = s.render()` and `fun g(s: Scope, render: (&Scope) -> Int): Bool = s.render()`
- **THEN** the file typechecks: the call in `f` calls the parameter `render`, and the call in `g` the extension, since a plain function type is not called in method form

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

#### Scenario: A receiver field through implicit this
- **WHEN** `struct User { val name: String, var nick: String }` and `mut fun String.clear()` are declared, and `mut fun User.tidy() { nick.clear() }` and `mut fun User.wipe() { name.clear() }` are checked
- **THEN** the type checker accepts `tidy` and emits `E4003` for `wipe`, naming the field `this.name` rather than a binding `name`

#### Scenario: Calling another extension on the implicit receiver
- **WHEN** `fun User.greet(): String = "Hi"` and `fun User.shout(): String = greet() + "!"` are declared
- **THEN** the file typechecks successfully, and `greet()` calls `this.greet()`

#### Scenario: A default value may read the receiver
- **WHEN** `struct User { val name: String }` and `fun User.greet(to: &String = name): String = "Hi " + to` are declared and `fun f(u: User): String = u.greet()` is checked
- **THEN** the file typechecks successfully

#### Scenario: this outside an extension
- **WHEN** `fun f() { val t = this }` is checked
- **THEN** the type checker emits `E4001`, saying that `this` is only available in an extension function or a lambda with a receiver

### Requirement: The Receiver in Lambdas With a Receiver
Inside a lambda with a receiver, `this` MUST be the lambda's receiver (`type-system`, "Lambdas With a Receiver"), and a bare name MUST resolve through it as through an extension's receiver. `this` outside both an extension body and a lambda with a receiver MUST be `E4001`, saying that `this` is only available in an extension function or a lambda with a receiver.

#### Scenario: Implicit this in a lambda with a receiver
- **WHEN** `struct Builder { var items: List<String> }`, `mut fun Builder.add(s: String) { items.add(s) }` and `fun build(f: mut Builder.() -> Unit): Int { var b = Builder(listOf()); b.f(); return b.items.size }` are declared, and `fun g(): Int = build { add("a"); add("b"); items.add("c") }` is checked
- **THEN** the file typechecks successfully: `add("a")` calls `this.add("a")` with a mutable reborrow of the lambda's receiver, and `items` reads `this.items`

### Requirement: Extension Declarations Are Checked
Two extensions in one file with the same receiver type constructor (`Box` for `Box<Int>` and `Box<T>`), name and context MUST be rejected with `E4010`. So MUST an extension that could never be called: one named like a field of its receiver struct, `clone` on a `Clone` type, or a member of its receiver type (`toString` on `Int`, `isEmpty` on `List`; `standard-library`). Extensions on different receivers or with different contexts, and an extension and a top-level function, MAY share a name.

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
- **WHEN** a file declares `fun Int.toString(): String = "x"`, `fun Char.toString(): String = "c"`, `fun String.length(): Int = 0` and `fun <T> List<T>.isEmpty(): Bool = true`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/core` always takes precedence

#### Scenario: Use before an inferred extension declaration
- **WHEN** `fun g(u: User): String { val s: String = u.size(); return s }` precedes `fun User.size() = 42`
- **THEN** the type checker emits `E4002`, exactly as when the declarations are swapped

### Requirement: Extension Receiver Types Follow Type Annotations
The receiver type of an extension MUST follow the rules of type annotations (`E4007`, `E4012`, `E4014`).

#### Scenario: Receiver type from an unimported module
- **WHEN** `shop/api` exports `public fun first(): Order`, where `Order` is declared in `shop/models`, and a file that imports only `/shop/api` declares `fun Order.total(): Int = 0`
- **THEN** the type checker emits `E4014` at the receiver type, naming `import /shop/models`

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

### Requirement: Only the Innermost Receiver Is Implicit
Bare names MUST resolve through the innermost `this` only: inside a lambda with a receiver, the receivers of enclosing extensions and lambdas MUST NOT be consulted for fields or extensions, and a name found only on one of them MUST be rejected with `E4001`, with help naming that outer receiver and suggesting a binding of it before the lambda.

#### Scenario: An outer receiver's field is hidden
- **WHEN** `struct User { val name: String }`, `struct Scope : Copy {}`, `fun column(f: Scope.() -> Unit) { f(Scope()) }` and `fun show(s: &String) {}` are declared, and `fun User.card() { column { show(name) } }` and `fun User.card2() { val user = this; column { show(user.name) } }` are checked
- **THEN** the type checker emits `E4001` at `name` in `card`, with help saying that `name` is a field of the outer receiver `User` and suggesting `val user = this` before the lambda, and accepts `card2`

#### Scenario: Nested scopes see their own extensions only
- **WHEN** `struct RowScope : Copy {}`, `struct BoxScope : Copy {}`, `fun row(f: RowScope.() -> Unit) { f(RowScope()) }`, `fun box(f: BoxScope.() -> Unit) { f(BoxScope()) }`, `fun RowScope.spacer() {}` and `fun BoxScope.overlay() {}` are declared, and `fun f() { row { spacer(); box { overlay() } } }` and `fun g() { row { box { spacer() } } }` are checked
- **THEN** the type checker accepts `f` and emits `E4001` at `spacer` in `g`, with help naming the outer receiver `RowScope`

### Requirement: Context Parameters
A function declared with a context parameter `context(name: T)` MUST be callable only where the call's context has a candidate of type `T`, and MUST receive that candidate, borrowed shared, as its context. Inside the function's body and defaults, `name` MUST be a `val` of type `&T`, and MUST NOT be an implicit receiver: bare names do not resolve through it.

#### Scenario: A context extension called in its scope
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy { val gap: Double }`, `fun row(f: RowScope.() -> Unit) { f(RowScope(4.0)) }`, `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w + scope.gap)` and `fun use(m: Modifier) {}` are declared, and `fun f() { row { use(Modifier(0.0).weight(1.0)) } }` is checked
- **THEN** the file typechecks successfully, and the call of `weight` receives the lambda's `this` as its context

#### Scenario: A context parameter is not a receiver
- **WHEN** `struct RowScope : Copy { val gap: Int }` is declared and `context(scope: RowScope) fun a(): Int = scope.gap` and `context(scope: RowScope) fun b(): Int = gap` are checked
- **THEN** the type checker accepts `a` and emits `E4001` at `gap` in `b`

#### Scenario: A plain function with a context
- **WHEN** `struct ColumnScope : Copy {}`, `fun column(f: ColumnScope.() -> Unit) { f(ColumnScope()) }` and `context(scope: ColumnScope) fun divider() {}` are declared, and `fun f() { column { divider() } }` and `fun g() { divider() }` are checked
- **THEN** the type checker accepts `f` and emits `E4034` at the call in `g`

### Requirement: A Call's Context Is the Innermost Receiver or the Enclosing Function
The context of a position MUST be the `this` of the innermost lambda with a receiver that encloses it within the current function, through any closures without a receiver; outside every such lambda, it MUST be the enclosing function's extension receiver and its context parameter. A call MUST take the first of these candidates whose type after references is exactly the context parameter's type, the extension receiver before the context parameter, and MUST NOT consult outer lambdas' receivers.

#### Scenario: Contexts from extension bodies and context functions
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}` and `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` are declared, and `fun RowScope.cell(): Modifier = Modifier(0.0).weight(1.0)` and `context(scope: RowScope) fun Modifier.half(): Modifier = weight(0.5)` are checked
- **THEN** the file typechecks successfully: `cell` passes its receiver as the context, and `half` its context parameter

#### Scenario: Closures without a receiver keep the context
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `fun row(f: RowScope.() -> Unit) { f(RowScope()) }`, `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` and `fun use(m: Modifier) {}` are declared, and `fun f(xs: &List<Double>) { row { xs.forEach { use(Modifier(0.0).weight(it)) } } }` is checked
- **THEN** the file typechecks successfully, the call taking the row lambda's `this` as its context

#### Scenario: An inner scope hides the outer context
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `struct BoxScope : Copy {}`, `fun row(f: RowScope.() -> Unit) { f(RowScope()) }`, `fun box(f: BoxScope.() -> Unit) { f(BoxScope()) }`, `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` and `fun use(m: Modifier) {}` are declared, and `fun f() { row { box { use(Modifier(0.0).weight(1.0)) } } }` and `context(scope: RowScope) fun g() { box { use(Modifier(0.0).weight(1.0)) } }` are checked
- **THEN** the type checker emits `E4034` at each call of `weight`, saying that it needs a `RowScope` and that the innermost receiver is a `BoxScope`

### Requirement: Extensions Differing Only by Context
Extensions of one receiver type constructor and name MAY coexist when their contexts differ, one of them possibly without a context. A call MUST resolve to the one whose context matches a candidate of the call's context, else to the one without a context. Two such extensions of one file with the same context (both none, or the same type) MUST be `E4010`, and so MUST two top-level functions of one name, whatever their contexts.

#### Scenario: The context chooses the extension
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `struct ColumnScope : Copy {}`, `fun row(f: RowScope.() -> Unit) { f(RowScope()) }`, `fun column(f: ColumnScope.() -> Unit) { f(ColumnScope()) }`, `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)`, `context(scope: ColumnScope) fun Modifier.weight(w: Double): Bool = true` and `fun use(m: Modifier) {}` are declared, and `fun f() { row { use(Modifier(0.0).weight(1.0)) }; column { val b: Bool = Modifier(0.0).weight(1.0) } }` is checked
- **THEN** the file typechecks successfully, the call in the row resolving to `RowScope`'s extension and the call in the column to `ColumnScope`'s

#### Scenario: An extension without a context is the fallback
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `fun row(f: RowScope.() -> Unit) { f(RowScope()) }`, `context(scope: RowScope) fun Modifier.tag(): Int = 1` and `fun Modifier.tag(): Bool = true` are declared, and `fun f(m: Modifier) { row { val a: Int = m.tag() }; val b: Bool = m.tag() }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Duplicate contexts rejected
- **WHEN** a file declares `struct Modifier : Copy {}`, `struct RowScope : Copy {}`, `context(scope: RowScope) fun Modifier.weight(): Int = 1`, `context(s: RowScope) fun Modifier.weight(): Int = 2`, `context(scope: RowScope) fun divider() {}` and `fun divider() {}`
- **THEN** the type checker emits `E4010` for the second `weight` and for the second `divider`

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

### Requirement: Extension Properties Are Read Like Fields
A read `value.name`, without a call, whose receiver's type after its `&`, `&mut` and `Heap<T>` layers has no field `name` MUST read a property: a member property of the type (`standard-library`, "Intrinsic Extensions Are Members"), else an extension property `name` of the file or of a direct import, found by receiver type constructor and name as an extension function is. The file's own MUST shadow imported ones; several imported MUST be `E4012`, and none `E4006`.

#### Scenario: Reading an extension property
- **WHEN** `struct Box : Copy { val w: Int, val h: Int }`, `val Box.area: Int get() = w * h` and `fun main() { val b = Box(2, 3); val r: &Box = &b; val none: Box? = null; println(b.area + r.area); println(none?.area ?: -1) }` run with `--check-allocations`
- **THEN** stdout is `12` then `-1`, and the exit status is 0

#### Scenario: An imported extension property
- **WHEN** `geo/shapes.prism` declares `package geo`, `public struct Box : Copy { val w: Int, val h: Int }` and `public val Box.area: Int get() = w * h`; a file importing `/geo/shapes` declares `fun f(b: Box): Int = b.area`; and a file importing it declares its own `val Box.area: Int get() = 0` and `fun g(b: Box): Int = b.area`
- **THEN** both files typecheck, the read in `f` calling `geo/shapes::Box.area` and the read in `g` the file's own property

#### Scenario: An unknown property
- **WHEN** a file declares `struct Box : Copy { val w: Int }` and `fun f(b: Box): Int = b.area`
- **THEN** the type checker emits `E4006` for `area`

### Requirement: An Extension Property Read Calls Its Getter
A read of an extension property MUST call its getter with a shared borrow of the receiver, as a method call with a shared receiver does, and have the getter's result as its value; `value?.name` MUST have type `R?`. The getter's type parameters MUST be solved from the receiver's type arguments alone. Assigning a property MUST be `E4003` (it has no setter), and calling it with parentheses MUST call the value read, as for a field.

#### Scenario: A generic extension property
- **WHEN** `struct Pair<T> { val a: T, val b: T }`, `val <T> Pair<T>.first: &T get() = &a` and `fun f(p: &Pair<String>): Int = p.first.length` are checked
- **THEN** the file typechecks, `p.first` having type `&String`, tied to `p` by the elision rules of a getter's shared receiver

#### Scenario: A property's value is the getter's result
- **WHEN** `struct Team { val members: List<String> }`, `val Team.first: String get() = members[0].clone()` and `val Team.names: List<String> get() = members.map { it.clone() }` are declared, with `import /prism/collections`, and `fun main() { val t = Team(listOf("ann", "bo")); val s = t.first; t.names.add("cy"); println(s + t.first); println(listOf(t).map { it.first }.size) }` runs with `--check-allocations`
- **THEN** stdout is `annann` then `1`, and the exit status is 0: each read is a temporary the caller owns, moved or changed without touching `t`

#### Scenario: Assigning an extension property rejected
- **WHEN** `struct Box : Copy { val w: Int }`, `val Box.twice: Int get() = w * 2` and `fun f() { var b = Box(1); b.twice = 4 }` are checked
- **THEN** the type checker emits `E4003` for the assignment, saying that `twice` has no setter

#### Scenario: Calling an extension property
- **WHEN** `struct Box : Copy { val w: Int }`, `val Box.twice: Int get() = w * 2` and `fun f(b: Box): Int = b.twice()` are checked
- **THEN** the type checker emits `E4004`, saying that `Int` cannot be called

### Requirement: Getter Bodies Are Extension Bodies
A getter's body MUST be checked as the body of an extension function with a shared receiver, the property's type parameters and no parameters, returning the property's type. `this`, a bare name reading the receiver's fields, members and extensions, `return`, and the elision rules for borrows a body returns apply as in such a function. A getter is not a composable context, and a reload swaps an edited getter's body at its next read, as a function's.

#### Scenario: A getter with a block body
- **WHEN** `struct Team { val members: List<String> }` and `val Team.label: String get() { if (members.isEmpty()) return "empty"; return members[0].clone() }` are declared and `fun main() { println(Team(listOf("ann", "bo")).label); println(Team(listOf()).label) }` runs with `--check-allocations`
- **THEN** stdout is `ann` then `empty`, and the exit status is 0

#### Scenario: A bare name reads an extension property
- **WHEN** `struct Box : Copy { val w: Int, val h: Int }`, `val Box.area: Int get() = w * h`, `val Box.double: Int get() = area * 2` and `fun Box.triple(): Int = area * 3` are declared and `fun main() { println(Box(2, 3).double); println(Box(2, 3).triple()) }` runs with `--check-allocations`
- **THEN** stdout is `12` then `18`, and the exit status is 0: in an extension or getter body, `area` reads `this.area`

#### Scenario: A getter calling a composable rejected
- **WHEN** `compose fun Leaf() {}`, `struct Box : Copy { val w: Int }` and `val Box.leaf: Int get() { Leaf(); return 1 }` are checked
- **THEN** the type checker emits `E4008` for the call of `Leaf`

#### Scenario: A getter edit reaches the next read
- **WHEN** `struct Box : Copy { val w: Int }`, `val Box.twice: Int get() = w * 2` and `fun main() { var i = 0; while (i < 4) { println(Box(i).twice); i += 1 } }` run under the script `after line 2: edit main.prism main.prism.v2`, where `main.prism.v2` declares the getter `get() = w * 10` and the same `main`
- **THEN** stdout holds `0`, `2`, `20` and `30`, and the exit status is 0: a getter is a function with a slot, swapped at its next call

### Requirement: Extension Property Declarations Are Checked
An extension property MUST be rejected with `E4010` when the file declares another extension property or extension function of the same name on the same receiver type constructor, when it is named like a field of its receiver struct or a variant of its receiver enum, or when it is named like a member of its receiver type (a member of a built-in type, or an intrinsic extension of a library type). An extension property and a top-level function or value MAY share a name.

#### Scenario: A property and a function of one name rejected
- **WHEN** a file declares `struct Box : Copy { val w: Int }`, `val Box.size: Int get() = w` and `fun Box.size(): Int = w`
- **THEN** the type checker emits `E4010` for the second declaration

#### Scenario: A property named like a field rejected
- **WHEN** a file declares `struct Box : Copy { val w: Int }` and `val Box.w: Int get() = 0`
- **THEN** the type checker emits `E4010`, saying that the field `w` always takes precedence

#### Scenario: A property named like a variant rejected
- **WHEN** a file declares `enum Color : Copy { Red, Green }` and `val Color.Red: Int get() = 1`
- **THEN** the type checker emits `E4010`, saying that the variant `Red` always takes precedence

#### Scenario: A property named like a member rejected
- **WHEN** a file declares `val String.length: Int get() = 0` and `val <T> List<T>.indices: Range get() = 0..<1`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/core` always takes precedence

### Requirement: Extension Property Reads Cost a Lookup Each
Checking a read of an extension property MUST cost what checking a method call with no arguments costs, at most two table lookups beyond the field and member tests, with no analysis of the getter's body at the read. Checking a file MUST grow near-linearly with its number of property reads and declarations.

#### Scenario: Many property reads check in linear time
- **WHEN** the `extension-properties` shape of `crates/prism-cli/tests/compile_timing.rs` checks files of 256, 1,024 and 4,096 functions, each reading a generic and a non-generic extension property of user types and a member property of `/prism/core`, with one declaration of each property per file, under `PRISM_ASSERT_TIMING=1`
- **THEN** each stage grows by at most ×4.6 per ×4 in size

## Design Rationale
Decisions from `extension-methods` (the core agreed with the user in the borrow-checker roadmap exploration, 2026-10-17; the rest decided while proposing the change and logged in `docs/decisions-for-review.md`):

- **Extensions are the only methods, declarable in any file** (user). One mechanism instead of Kotlin's member-versus-extension precedence; built-in and user types gain methods the same way; and a struct's ABI stays its layout and markers, so adding a method in another file never changes the struct's ABI hash. Dispatch is static, from the receiver's static type. Rejected: member functions in struct bodies, which put methods into the struct's ABI, and Go's rule that methods live in the type's package, which mainly protects trait coherence and can be added for conformances when traits exist. Costs: no encapsulation boundary until field visibility exists, and trait conformances will need their own block.
- **Members first, then the file's own extensions, then imports** (user for shadowing and `E4012`). A field wins over an extension, as a field named `clone` does over the built-in, and the built-in `clone()` of a `Clone` type wins over an extension (members before extensions, as in Kotlin). Extensions live in their own table keyed by receiver type and name, apart from the name scopes, so `fun greet()` and `fun User.greet()` coexist, extensions on different receivers never collide, and ambiguity is per receiver type and name. Since `standard-library`, a workspace file's lookup ended with the prelude's extensions, after its own and its imports', so either shadowed a library helper without `E4012`. Since `explicit-library` there is no prelude layer: a library extension is found only through an import of its module, like any imported extension, so a workspace module's extension of the same receiver and name beside that import is `E4012` at use, and the file's own extension shadows both (`explicit-library` D1, D4).
- **Implicit `this`: locals, then receiver fields, then receiver extensions, then the module** (user for fields; extensions autonomous). A bare name is rewritten to the explicit `this.name` before it is checked, so reads, writes, borrows, moves, method calls and their diagnostics all go through the ordinary member rules. Calling a sibling extension without `this.` follows Kotlin and Swift. Rejected: fields only, and module declarations before receiver members, which would make a field unreachable by its bare name whenever a top-level declaration shares it.
- **Uncallable extensions are rejected with `E4010`.** An extension named like a field of its receiver, or named `clone` on a `Clone` type, could never be called. Rejecting it at the declaration was preferred to accepting it silently (Kotlin warns, and Prism has no warnings channel): a struct that gains a field would otherwise silently redirect every call to the field. Reusing `E4010` follows Swift, which reports such a clash as an invalid redeclaration.
- **`?.` calls give `R?`, and `?.` chains of fields over a place are optional places for receivers.** `t?.lead?.greet()` borrows `lead` where it lives rather than moving it into a temporary, which would reject the most common Kotlin method-call idiom. This is safe for receivers because the call borrows the payload `&User`, never a `User?`. Nullable receiver types (`fun String?.orEmpty()`) were deferred, since they need a rule for which of `T` and `T?` a call picks.
- **Calling an imported extension does not need the receiver type's module imported.** The call is a function of the extension's module, whose ABI already binds the receiver type, and a value of the type can already be received and passed along without `E4014`.
- **`this` is a `val` in every mode.** Parameters are immutable everywhere, so a `consuming fun` moves `this` into a `var` to mutate it. It is its own expression node, not an identifier, and is bound before the parameters so defaults may read it.
- **Unannotated private extensions are inferred on demand, keyed by receiver type and name,** so `User.size` and `Team.size` are inferred separately.
- **No new error codes.** `this` outside an extension is an unresolved name (`E4001`), a `mut fun` receiver that is not a mutable place is `E4003`, an unknown method or an extension read without a call is `E4006`, and the other cases are the kinds of error their codes already name.

Decisions from `runtime-strings-and-drops` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Built-in members win over extensions** (D6). `length` counts as a field of `String` and `toString()` is a member of `Int`, `Double`, `Bool` and `Char`, so, like `clone()` on a `Clone` type, they win over extensions, and an extension named `length` on `String` or `toString` on those four types could never be called and is `E4010`. On every other type `toString` is an ordinary extension name. Since `standard-library` these members, and the list members, are the `intrinsic` extensions of `/prism/core` and `/prism/collections`, read from their declarations rather than a hand-written list; since `core-library-in-prism`, `/prism/core` alone declares them, intrinsic or written in Prism.

Decisions from `receiver-lambdas` (planned autonomously on 2026-10-19 for `ui-toolkit`'s Compose-like layout scopes, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Implicit `this` resolves against the innermost receiver only** (D4). Inside a receiver lambda, bare names resolve through the lambda's `this` exactly as in an extension body; an enclosing extension's or lambda's receiver is not consulted. This is Compose's `@LayoutScopeMarker` behaviour applied to every receiver, at constant cost per name, where Kotlin's chain of implicit receivers costs time in the nesting depth per unresolved name and needs a marker annotation. A name found only on an outer receiver is `E4001` (`OuterReceiverName`), whose help, computed only on that error path, names the receiver and suggests `val user = this` before the lambda. Rejected: Kotlin's receiver chain with a `@DslMarker`-like modifier, the chain without markers (outer layout scopes would leak into inner ones), and labelled `this@name` for now (a follow-up).
- **A call's context is the innermost receiver lambda's `this`, or the enclosing function's receiver and context parameter** (D7). The checker keeps the current context as the bindings of at most two candidates, replaced on entering a receiver lambda or a function body, so a context call costs at most two type comparisons and a local shadowing a context parameter's name never becomes the context argument. Closures without a receiver keep the enclosing context. Rejected: searching every enclosing receiver (D4), and explicit `with(scope) { ... }` blocks (not Compose's surface).
- **Context parameters are not receivers** (D6). Inside the body and the defaults the parameter is a `val` of type `&T`, passed as the function's first parameter, before the receiver, as a shared borrow (a scope may one day hold state, and a shared borrow is what a `fun` receiver gets). Rejected: Kotlin's withdrawn context receivers, which put two implicit receivers at every position, and passing the context by value.
- **Extensions may differ only by context, resolved layer by layer** (D8). Each extension table key (receiver constructor and name) holds at most one context-free extension and one per context type, indexed by that type, so resolving among many contexts is a lookup, not a scan. A call picks, within the first layer (own, imports; a prelude layer until `explicit-library`) that has the key, the extension whose context matches a candidate, else the context-free one, else `E4034` naming the contexts that layer declares. Rejected: choosing across layers (the file's own declaration would no longer shadow imports), preferring the context-free extension (a scoped `weight` could never be called beside a plain one), and overloading plain functions by context (the name scopes hold one value per name; a follow-up).
- **A missing context is its own error, `E4034`** (D10). It names the context the function needs, the one the call has (the innermost receiver lambda's `this`, the enclosing extension's receiver, or none) and, when there are several, the contexts the extension is declared for, with help to call it inside a matching receiver lambda or declare the enclosing function `context(...)`. Naming a plain context function as a value is `E4034` too, since its value would need its context. Rejected: `E4006`, which hides that the extension exists and what it needs. The code follows those `compose-transform` and `state-handles` reserve (`E4030` to `E4033`), so the packets keep their codes in any order.

Decisions from `core-library-in-prism` (the core half of the review's agreed item A4, user direction 2026-10-20 for writing the library in Prism over the smallest intrinsic core with no `unsafe`; planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Read-only extension properties with a getter, Kotlin's form, in every file** (D5). `val Receiver.name: Type get() = expression` or `get() { ... }`, the getter on the header's line or the next, `get` a keyword only there. The type is always written, so Phase 1 needs nothing new; type parameters must occur in the receiver, so a read solves them from it alone; an initializer, `var`, `compose`, a receiver mode, and (found at review) a `Heap<...>` or bare type-parameter receiver are `E2001`. Setters, `compose` and context-parameter properties are deferred. Rejected: keeping `intrinsic val` as the only form, unit functions (`16.dp()`), an initializer form (it reads as a value computed once), and library-only properties.
- **An extension property is an extension function read without parentheses** (D6). Properties share the extension table with functions (one entry per receiver constructor, name and context), so a property and a function of one name in one file are `E4010`, as are properties named like a field or a member and, found at review, like a variant of the receiver enum, which `c.Red` on a value always names. A read looks up a field, then a member property, then the file's own extension properties, then its imports' (`E4012`, `E4006`); `x.p()` calls the value read; an assignment is `E4003`, saying that the property has no setter. The getter is a function `<module>::<Receiver>.<name>` with a shared receiver, checked as an extension body, so the typed tree, MIR, the borrow checker, code generation and hot reload see an ordinary extension call and nothing after the type checker knows properties. Cost: a failed field test, one member lookup and at most two extension-table lookups per read (shape `extension-properties`, near-linear). Rejected: a separate property namespace (two lookups per read and call for a confusing surface), and a property node in the typed tree.
- **Reads behave as Kotlin's: temporaries, and bare names through the receiver** (D16). A read is a temporary the caller owns: `val s = t.first` moves the getter's result, not a field, and `t.names.add(x)` changes the temporary list, while a getter returning `&T` gives a read-only borrow tied to the receiver. A bare name in an extension or getter body reads a property of the innermost receiver, as it reads a field. In a composable, a borrowed argument built from a property read is a call's result, so it has no identity (version `0`, `compose`) and runs its callee in a replaceable group, never skipped, as any call's result does; `Copy` results compare by `==` as usual, and the examples read fields or `Copy` units only. Rejected: treating a read as a field for moves and writes (`xs.map { it.first }` would be `E4017`), and requiring `this.` before a property.

Decisions from `field-visibility` (item 10b of the review round, the language prerequisite of the compose half of the review's agreed item A4, user direction 2026-10-20 for library handles written as ordinary Prism structs; planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Only visible fields resolve** (D5). Call precedence, property reads, bare names and the `E4010` rule for an extension or extension property named like a field count only fields visible in the file, so another file can declare `fun Session.id(): Int` for a struct with a `private val id` and call it, while in the struct's own file the field still wins and such an extension is still `E4010`. The field lookup gains one comparison. Rejected: hidden fields shadowing extensions everywhere, which would let a file declare an extension it can never call.
