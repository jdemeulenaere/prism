# Extension Functions Specification Delta

## ADDED Requirements

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
Extensions of one receiver type constructor and name MAY coexist when their contexts differ, one of them possibly without a context. A call MUST resolve to the one whose context matches a candidate of the call's context, else to the one without a context.

#### Scenario: The context chooses the extension
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `struct ColumnScope : Copy {}`, `fun row(f: RowScope.() -> Unit) { f(RowScope()) }`, `fun column(f: ColumnScope.() -> Unit) { f(ColumnScope()) }`, `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)`, `context(scope: ColumnScope) fun Modifier.weight(w: Double): Bool = true` and `fun use(m: Modifier) {}` are declared, and `fun f() { row { use(Modifier(0.0).weight(1.0)) }; column { val b: Bool = Modifier(0.0).weight(1.0) } }` is checked
- **THEN** the file typechecks successfully, the call in the row resolving to `RowScope`'s extension and the call in the column to `ColumnScope`'s

#### Scenario: An extension without a context is the fallback
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `fun row(f: RowScope.() -> Unit) { f(RowScope()) }`, `context(scope: RowScope) fun Modifier.tag(): Int = 1` and `fun Modifier.tag(): Bool = true` are declared, and `fun f(m: Modifier) { row { val a: Int = m.tag() }; val b: Bool = m.tag() }` is checked
- **THEN** the file typechecks successfully

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

## MODIFIED Requirements

### Requirement: The Receiver in Extension Bodies
Inside an extension body, `this` MUST be the receiver, a `val` binding of type `&T` in a `fun`, `&mut T` in a `mut fun` and `T` in a `consuming fun`, in scope in the body and in parameter defaults. Inside a lambda with a receiver, `this` MUST be the lambda's receiver (`type-system`, "Lambdas With a Receiver"), and a bare name MUST resolve through it as through an extension's. A bare name that is not a local or parameter MUST resolve to `this.name` when the receiver's struct has that field, and, when called, to `this.name(...)` when an extension on the receiver type matches, before top-level declarations and imports. `this` elsewhere MUST be `E4001`, saying that `this` is only available in an extension function or a lambda with a receiver.

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

#### Scenario: Implicit this in a lambda with a receiver
- **WHEN** `struct Builder { var items: List<String> }`, `mut fun Builder.add(s: String) { items.add(s) }` and `fun build(f: mut Builder.() -> Unit): Int { var b = Builder(listOf()); b.f(); return b.items.size }` are declared, and `fun g(): Int = build { add("a"); add("b"); items.add("c") }` is checked
- **THEN** the file typechecks successfully: `add("a")` calls `this.add("a")` with a mutable reborrow of the lambda's receiver, and `items` reads `this.items`

### Requirement: Members Take Precedence Over Extensions
A call `value.name(...)` MUST invoke a field `name` of the receiver's struct, else the built-in `clone()` of a `Clone` receiver or `unbox()` of a `Heap<T>` behind its references, else an intrinsic extension `name` on the receiver's type (`standard-library`, "Intrinsic Extensions Are Members"), such as a scalar's `toString()` or a `List<T>` member, whatever extensions are visible. An intrinsic property (`length`, `size`, `indices`) counts as a field. Then, when the innermost local or parameter named `name` has a function type with a receiver whose receiver type is the receiver's type, the call MUST call that value with the receiver (`type-system`, "Calling Function Values With a Receiver"). Only then MUST an extension be considered.

#### Scenario: A field wins over an extension
- **WHEN** a file declares `struct Button { val onClick: () -> String }` and `fun Button.onClick(): Int = 1`, and `fun f(b: Button) { val s: String = b.onClick() }`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, because the call invokes the field

#### Scenario: A built-in toString wins over an extension
- **WHEN** a file declares `fun Int.toString(): String = "x"` and `fun f(n: Int): String = n.toString()`
- **THEN** the type checker emits `E4010` for the extension and nothing for `f`, whose call is the intrinsic `toString()` of `/prism/core`

#### Scenario: The built-in unbox wins over an extension on the contents
- **WHEN** a file declares `struct Gift { val n: Int }`, `fun Gift.unbox(): Int = n`, `fun f(h: Heap<Gift>): Gift = h.unbox()` and `fun g(h: Heap<Gift>): Int { val r: &Gift = &h; return r.unbox() }`
- **THEN** the file typechecks: the call in `f` is the built-in `unbox()` of the box, and the call in `g` is the extension, on the borrowed contents

#### Scenario: A receiver function value wins over an extension
- **WHEN** a file declares `struct Scope : Copy {}`, `fun Scope.render(): Bool = true`, `fun f(s: Scope, render: Scope.() -> Int): Int = s.render()` and `fun g(s: Scope, render: (&Scope) -> Int): Bool = s.render()`
- **THEN** the file typechecks: the call in `f` calls the parameter `render`, and the call in `g` the extension, since a plain function type is not called in method form

### Requirement: Extension Declarations Are Checked
Two extensions in one file with the same receiver type constructor (`Box` for `Box<Int>` and `Box<T>`), name and context (both without one, or both with the same type) MUST be rejected with `E4010`, as MUST an extension that could never be called: one named like a field of its receiver struct, named `clone` on a `Clone` type, named `toString` on `Int`, `Double`, `Bool` or `Char`, or named `length` on `String`. Extensions on different receivers or with different contexts, and an extension and a top-level function, MAY share a name. Two top-level functions of one name MUST be rejected with `E4010` whatever their contexts.

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

#### Scenario: Duplicate contexts rejected
- **WHEN** a file declares `struct Modifier : Copy {}`, `struct RowScope : Copy {}`, `context(scope: RowScope) fun Modifier.weight(): Int = 1`, `context(s: RowScope) fun Modifier.weight(): Int = 2`, `context(scope: RowScope) fun divider() {}` and `fun divider() {}`
- **THEN** the type checker emits `E4010` for the second `weight` and for the second `divider`
