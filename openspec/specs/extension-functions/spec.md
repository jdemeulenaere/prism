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

### Requirement: Extension Calls Resolve Through the File and Its Imports
A call `value.name(...)` MUST resolve to an extension `name` on the value's type after `&`, `&mut` and `Heap<T>` layers, declared in the file, exported by a direct import or, in a workspace file, by a prelude module (`standard-library`). The file's own match MUST shadow imported ones, which shadow a prelude one. Several imported matches MUST be `E4012` at use, naming their modules, and none `E4006`. Calling an imported extension MUST NOT need an import of its receiver type's module.

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
A call `value.name(...)` MUST invoke a field `name` of the receiver's struct, else the built-in `clone()` of a `Clone` receiver or `unbox()` of a `Heap<T>` behind its references, else an intrinsic extension `name` on its type (`standard-library`), such as a scalar's `toString()` or a `List<T>` member, whatever extensions are visible; an intrinsic property (`length`, `size`, `indices`) counts as a field. Then it MUST call a receiver function value `name`, and only then an extension.

#### Scenario: A field wins over an extension
- **WHEN** a file declares `struct Button { val onClick: () -> String }` and `fun Button.onClick(): Int = 1`, and `fun f(b: Button) { val s: String = b.onClick() }`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, because the call invokes the field

#### Scenario: A built-in toString wins over an extension
- **WHEN** a file declares `fun Int.toString(): String = "x"` and `fun f(n: Int): String = n.toString()`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, whose call is the intrinsic `toString()` of `/prism/core`

#### Scenario: The built-in unbox wins over an extension on the contents
- **WHEN** a file declares `struct Gift { val n: Int }`, `fun Gift.unbox(): Int = n`, `fun f(h: Heap<Gift>): Gift = h.unbox()` and `fun g(h: Heap<Gift>): Int { val r: &Gift = &h; return r.unbox() }`
- **THEN** the file typechecks: the call in `f` is the built-in `unbox()` of the box, and the call in `g` is the extension, on the borrowed contents

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
Two extensions in one file with the same receiver type constructor (`Box` for `Box<Int>` and `Box<T>`), name and context MUST be rejected with `E4010`, as MUST an extension that could never be called: one named like a field of its receiver struct, named `clone` on a `Clone` type, named `toString` on `Int`, `Double`, `Bool` or `Char`, or named `length` on `String`. Extensions on different receivers or with different contexts, and an extension and a top-level function, MAY share a name.

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
A call MUST resolve in the first layer (the file's own extensions, then the imports', then the prelude's) that declares an extension of its receiver and name, by the rule of "Extensions Differing Only by Context", and MUST be rejected with `E4034` when that layer has no match, naming the contexts it declares. Several imports exporting a matching extension of one receiver, name and context MUST be `E4012`.

#### Scenario: The file's own extension shadows an imported context extension
- **WHEN** `layout/scope.prism` exports `public struct Modifier : Copy { val fraction: Double }`, `public struct RowScope : Copy {}`, `public fun row(f: RowScope.() -> Unit): Unit { f(RowScope()) }` and `context(scope: RowScope) public fun Modifier.weight(w: Double): Modifier = Modifier(w)`, and a file importing `/layout/scope` declares `fun Modifier.weight(w: Double): Bool = true` and `fun f() { row { val b: Bool = Modifier(0.0).weight(1.0) } }`
- **THEN** the file typechecks successfully, the call resolving to the file's own extension

#### Scenario: Missing context names the declared contexts
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `struct ColumnScope : Copy {}`, `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` and `context(scope: ColumnScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` are declared and `fun f(): Modifier = Modifier(0.0).weight(1.0)` is checked
- **THEN** the type checker emits `E4034` at the call, saying that `weight` is declared for the contexts `RowScope` and `ColumnScope` and that this position has no receiver

#### Scenario: Ambiguous imported context extensions
- **WHEN** `ui/scope.prism` exports `public struct Modifier : Copy { val fraction: Double }`, `public struct RowScope : Copy {}`, `public struct ColumnScope : Copy {}` and `public fun row(f: RowScope.() -> Unit): Unit { f(RowScope()) }`, `a/lib` and `b/lib` each import `/ui/scope` and export `context(scope: RowScope) public fun Modifier.weight(w: Double): Modifier = Modifier(w)`, and a file importing `/ui/scope`, `/a/lib` and `/b/lib` declares `fun f() { row { val m = Modifier(0.0).weight(1.0) } }`
- **THEN** the type checker emits `E4012` naming `a/lib` and `b/lib`, and the file typechecks successfully when `b/lib`'s extension is declared for the context `ColumnScope` instead

## Design Rationale
Decisions from `extension-methods` (the core agreed with the user in the borrow-checker roadmap exploration, 2026-10-17; the rest decided while proposing the change and logged in `docs/decisions-for-review.md`):

- **Extensions are the only methods, declarable in any file** (user). One mechanism instead of Kotlin's member-versus-extension precedence; built-in and user types gain methods the same way; and a struct's ABI stays its layout and markers, so adding a method in another file never changes the struct's ABI hash. Dispatch is static, from the receiver's static type. Rejected: member functions in struct bodies, which put methods into the struct's ABI, and Go's rule that methods live in the type's package, which mainly protects trait coherence and can be added for conformances when traits exist. Costs: no encapsulation boundary until field visibility exists, and trait conformances will need their own block.
- **Members first, then the file's own extensions, then imports** (user for shadowing and `E4012`). A field wins over an extension, as a field named `clone` does over the built-in, and the built-in `clone()` of a `Clone` type wins over an extension (members before extensions, as in Kotlin). Extensions live in their own table keyed by receiver type and name, apart from the name scopes, so `fun greet()` and `fun User.greet()` coexist, extensions on different receivers never collide, and ambiguity is per receiver type and name. Since `standard-library`, a workspace file's lookup ends with the prelude's extensions, after its own and its imports', so either shadows a library helper without `E4012`.
- **Implicit `this`: locals, then receiver fields, then receiver extensions, then the module** (user for fields; extensions autonomous). A bare name is rewritten to the explicit `this.name` before it is checked, so reads, writes, borrows, moves, method calls and their diagnostics all go through the ordinary member rules. Calling a sibling extension without `this.` follows Kotlin and Swift. Rejected: fields only, and module declarations before receiver members, which would make a field unreachable by its bare name whenever a top-level declaration shares it.
- **Uncallable extensions are rejected with `E4010`.** An extension named like a field of its receiver, or named `clone` on a `Clone` type, could never be called. Rejecting it at the declaration was preferred to accepting it silently (Kotlin warns, and Prism has no warnings channel): a struct that gains a field would otherwise silently redirect every call to the field. Reusing `E4010` follows Swift, which reports such a clash as an invalid redeclaration.
- **`?.` calls give `R?`, and `?.` chains of fields over a place are optional places for receivers.** `t?.lead?.greet()` borrows `lead` where it lives rather than moving it into a temporary, which would reject the most common Kotlin method-call idiom. This is safe for receivers because the call borrows the payload `&User`, never a `User?`. Nullable receiver types (`fun String?.orEmpty()`) were deferred, since they need a rule for which of `T` and `T?` a call picks.
- **Calling an imported extension does not need the receiver type's module imported.** The call is a function of the extension's module, whose ABI already binds the receiver type, and a value of the type can already be received and passed along without `E4014`.
- **`this` is a `val` in every mode.** Parameters are immutable everywhere, so a `consuming fun` moves `this` into a `var` to mutate it. It is its own expression node, not an identifier, and is bound before the parameters so defaults may read it.
- **Unannotated private extensions are inferred on demand, keyed by receiver type and name,** so `User.size` and `Team.size` are inferred separately.
- **No new error codes.** `this` outside an extension is an unresolved name (`E4001`), a `mut fun` receiver that is not a mutable place is `E4003`, an unknown method or an extension read without a call is `E4006`, and the other cases are the kinds of error their codes already name.

Decisions from `runtime-strings-and-drops` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Built-in members win over extensions** (D6). `length` counts as a field of `String` and `toString()` is a member of `Int`, `Double`, `Bool` and `Char`, so, like `clone()` on a `Clone` type, they win over extensions, and an extension named `length` on `String` or `toString` on those four types could never be called and is `E4010`. On every other type `toString` is an ordinary extension name. Since `standard-library` these members, and the list members, are the `intrinsic` extensions of `/prism/core` and `/prism/collections`, read from their declarations rather than a hand-written list.

Decisions from `receiver-lambdas` (planned autonomously on 2026-10-19 for `ui-toolkit`'s Compose-like layout scopes, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Implicit `this` resolves against the innermost receiver only** (D4). Inside a receiver lambda, bare names resolve through the lambda's `this` exactly as in an extension body; an enclosing extension's or lambda's receiver is not consulted. This is Compose's `@LayoutScopeMarker` behaviour applied to every receiver, at constant cost per name, where Kotlin's chain of implicit receivers costs time in the nesting depth per unresolved name and needs a marker annotation. A name found only on an outer receiver is `E4001` (`OuterReceiverName`), whose help, computed only on that error path, names the receiver and suggests `val user = this` before the lambda. Rejected: Kotlin's receiver chain with a `@DslMarker`-like modifier, the chain without markers (outer layout scopes would leak into inner ones), and labelled `this@name` for now (a follow-up).
- **A call's context is the innermost receiver lambda's `this`, or the enclosing function's receiver and context parameter** (D7). The checker keeps the current context as the bindings of at most two candidates, replaced on entering a receiver lambda or a function body, so a context call costs at most two type comparisons and a local shadowing a context parameter's name never becomes the context argument. Closures without a receiver keep the enclosing context. Rejected: searching every enclosing receiver (D4), and explicit `with(scope) { ... }` blocks (not Compose's surface).
- **Context parameters are not receivers** (D6). Inside the body and the defaults the parameter is a `val` of type `&T`, passed as the function's first parameter, before the receiver, as a shared borrow (a scope may one day hold state, and a shared borrow is what a `fun` receiver gets). Rejected: Kotlin's withdrawn context receivers, which put two implicit receivers at every position, and passing the context by value.
- **Extensions may differ only by context, resolved layer by layer** (D8). Each extension table key (receiver constructor and name) holds at most one context-free extension and one per context type, indexed by that type, so resolving among many contexts is a lookup, not a scan. A call picks, within the first layer (own, imports, prelude) that has the key, the extension whose context matches a candidate, else the context-free one, else `E4034` naming the contexts that layer declares. Rejected: choosing across layers (the file's own declaration would no longer shadow imports), preferring the context-free extension (a scoped `weight` could never be called beside a plain one), and overloading plain functions by context (the name scopes hold one value per name; a follow-up).
- **A missing context is its own error, `E4034`** (D10). It names the context the function needs, the one the call has (the innermost receiver lambda's `this`, the enclosing extension's receiver, or none) and, when there are several, the contexts the extension is declared for, with help to call it inside a matching receiver lambda or declare the enclosing function `context(...)`. Naming a plain context function as a value is `E4034` too, since its value would need its context. Rejected: `E4006`, which hides that the extension exists and what it needs. The code follows those `compose-transform` and `state-handles` reserve (`E4030` to `E4033`), so the packets keep their codes in any order.
