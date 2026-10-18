# Design: receiver-lambdas

## Context

See `proposal.md` for motivation. This change was added to the road to Milestone 2 on 2026-10-19 (process row P13 of `docs/decisions-for-review.md`), when the user asked `ui-toolkit` to follow Jetpack Compose's APIs; `ui-toolkit` D22 sets its scope and leaves the details and the syntax to this packet. It is planned autonomously: every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19), so each decision states its compile-time cost next to its Kotlin-likeness.

Current state:

- **Function types.** `TypeKind::Function { is_compose, params, escaping, return_type }` in the AST and `Type::Fn { is_compose, params, escaping, ret }` in the typed tree. `parse_type_inner` parses `compose (` and `(` as function types and a named type with optional type arguments, then a `?` suffix; no type contains a `.`. `escaping` is a modifier before `(`, `compose`, `&` and identifiers (`at_escaping_before_type`). `Type::converts_in_place` (D8 of `closures`, the type-system rationale "Function values convert only without changing representation") allows a function value to convert only when its parameters and result are equal or weaken `&mut T` to `&T`, compose flags are equal and parameters may gain `escaping`.
- **Extensions.** `FunctionDecl::receiver: Option<Receiver { mode, ty, span }>` with `ReceiverMode::{Shared, Mutable, Consuming}`. In a body, `this` is a scope symbol `THIS` of type `&T`, `&mut T` or `T`; `current_receiver` reads the innermost `THIS`, and `implicit_this_field`/`implicit_this_callee` rewrite a bare name that is no local into `this.name` when the receiver has that field, intrinsic member or extension. `this` with no `THIS` in scope is `ThisOutsideExtension` (`E4001`). Extensions live in `own_extensions` and `imported_extensions`, keyed by `(ReceiverKey, name)`, with the prelude last (`find_extension`); a second own extension of one key is `E4010`; several imported ones are `E4012` at use.
- **Closures.** `check_closure` checks a closure against an expected function type: it binds the declared parameters, or the implicit `it` when the closure has no header and one parameter is expected (`implicit-it`), in the scope `enter_closure` opens, and checks the body in the expected type's composable context. Captures are recorded at use; `this` used in a closure inside an extension body is a capture of the extension's `this`.
- **Calls of function values** go through `check_call_arguments` with the value's parameter types; a `&T` parameter borrows its argument implicitly, a `&mut T` one needs `&mut` written. A method call `value.name(...)` classifies its receiver first, then resolves a field, an intrinsic member, then an extension (`find_extension_through_indirection`).
- **Downstream.** The typed tree normalizes an extension call to a call whose first argument is the receiver; MIR's operands lead with it; the backend calls a function value through its descriptor's slot with the environment and the arguments. `compose-transform` (proposed) gives composables, composable closures and composable function types the composer after their declared parameters, and change words for every parameter, "the receiver first" (its D3 and D4).
- **Empty structs** (`struct Marker : Copy {}`) are allowed, zero-sized, and boxes and lists of them allocate nothing (fixtures `a-box-of-a-zero-sized-value`, `empty-lists-and-lists-of-zero-sized-elements-allocate-nothing`). Scopes such as `public struct ColumnScope : Copy {}` are such structs.

## Goals / Non-Goals

**Goals:**
- Kotlin's function literals with receiver and Kotlin 2.2's context parameters, to the extent Compose's layout scopes and builder DSLs use them, with Compose's surface: `Column { Text("a", Modifier.weight(1.0)) }`.
- Constant work per lambda and per call: no inference from bodies, no search through outer scopes, no re-checking.
- No new behaviour after the type checker: a receiver lambda is a closure with one more parameter, a context argument is one more argument.
- A fixture for every scenario, executable ones (a hand-written `Column`-like DSL, a builder) under `--check-allocations`.

**Non-Goals:**
- `consuming` receivers in function types (D1), several context parameters (D6), labelled `this@name` (D4), Kotlin's `@DslMarker` (D4), plain functions overloaded by context (D8), context parameters on library declarations (D6), extensions on a bare type parameter.
- Composable receiver lambdas *running*: they need `compose-transform`, which sees them as composable closures with a first parameter (D12). This change checks them.

## Decisions

### D1. `T.(A) -> R` and `mut T.(A) -> R` (autonomous: the `mut` form, type-parameter receivers, no `consuming`)

A function type may name a receiver before its parameter list: `ColumnScope.() -> Unit`, `List<Int>.(Int) -> Bool`, `T.() -> R` for a type parameter `T` in scope. `mut` before the receiver makes it a mutable receiver: `mut Builder.(String) -> Unit`. The prefixes keep the order of declarations (`compose mut fun T.name()`): `compose mut Builder.() -> Unit`, and a parameter's `escaping` comes first, `escaping compose ColumnScope.() -> Unit`. A nullable receiver function type needs parentheses, `(ColumnScope.() -> Unit)?`, as any function type.

The receiver is parsed as an extension receiver is, with one difference: a type parameter is allowed, since a lambda checked against `T.() -> R` is checked after `T` is known (`fun <T> with(receiver: &T, block: T.() -> Unit)` is Kotlin's `with`). A `?`, `&`, `&mut`, function, parenthesized or `Heap<...>` receiver is `E2001`, with the extension receivers' messages. The parser decides with one token of lookahead: a named type followed by `.(` is a receiver, and `mut` cannot otherwise start a type.

**Compile-time cost and Kotlin-likeness.** One lookahead token per named type. Kotlin's syntax exactly for shared receivers; `mut` has no Kotlin counterpart, because Kotlin has no borrows.

Rejected:
- **Only shared receivers**: builder DSLs (`buildList { add(x) }`, a `Column`-like builder collecting children) need to mutate their receiver, and a `&mut` parameter would force `{ b -> b.add(x) }`, losing the implicit `this`.
- **`(&mut T).() -> R` or `&mut T.() -> R`** for the mutable form: references as receivers are rejected for extensions, and the receiver mode is written as a modifier there (`mut fun`), so the type mirrors it.
- **`consuming T.() -> R`**: no use case yet; a follow-up can add it with the declaration modifier's meaning.

### D2. The receiver is the first parameter; receiver and plain function types convert in place (autonomous)

`Type::Fn` gains `receiver: bool`. When it is set, `params[0]` is the receiver's parameter type, `&T` for a shared receiver and `&mut T` for a mutable one, and `escaping[0]` is false; the other parameters follow. So `ColumnScope.(Int) -> Unit` has the parameters `&ColumnScope, Int`, and everything that walks parameters (borrow checker, MIR, backend, `compose-transform`'s change words) needs no change. The flag is part of the type's identity and of its display (`ColumnScope.(Int) -> Unit`), since it decides how a lambda is checked against it.

`converts_in_place` ignores the flag: `T.(A) -> R` and `(&T, A) -> R` have one representation, so they convert into each other in both directions, as Kotlin lets `T.() -> R` and `(T) -> R` stand for each other. The other rules apply to the receiver's parameter as to any: `T.() -> R` converts to `mut T.() -> R` (a function that only reads its receiver may be given a mutable one), never the reverse. So `fun header(scope: &ColumnScope) {}` can be passed as a `ColumnScope.() -> Unit`, and a content value can be handed to code written for `(&ColumnScope) -> Unit`.

**Compile-time cost.** One flag per function type, compared with the other fields. The conversion rule is unchanged in cost.

Rejected:
- **No conversion** between the two forms: passing a value of type `(&ColumnScope) -> Unit` as content would need a calling closure, unlike Kotlin.
- **One type, the flag outside identity** (a lambda gets `this` when its expected type was *written* with a receiver): the written form would have to travel through inference and generic substitution beside the type, which is more state for the same result.
- **The receiver as a separate field beside `params`**: every consumer that counts or walks parameters would need to handle it.

### D3. A lambda against a receiver function type binds `this` (autonomous: the local kind)

When `check_closure`'s expected type has a receiver, the closure binds `this` in its own scope, before its parameters, as a `val` of the receiver's parameter type (`&T` or `&mut T`), declared at the closure's `{`. Its header declares the other parameters, and the implicit `it` applies to them: a header-less `T.(A) -> R` lambda has `it: A`, a `T.() -> R` lambda has none, and the arity diagnostics of `implicit-it` count the other parameters only. Every expected-type source works: a parameter (a trailing lambda included), an annotation, a return type, a field, a deferred closure of a generic call once its type arguments are known, a nullable function type.

A closure with no expected type, or one checked against a plain function type, has no receiver: `this` in it is the enclosing extension's or receiver lambda's (a capture), or `E4001` with no enclosing one. The `E4001` message for `this` becomes "'this' is only available in an extension function or a lambda with a receiver".

In the typed tree the receiver is the closure's first parameter, a local named `this` of the new kind `LocalKind::ClosureReceiver`, dumped as `(closure receiver)`; uses of `this` and of bare names through it are reads of that local.

**Compile-time cost.** One more binding per receiver lambda, made where `check_closure` already binds parameters. Kotlin's behaviour exactly.

Rejected: rewriting the lambda to `{ this -> ... }` in the parser (it does not know the expected type, and `this` is a keyword); reusing `LocalKind::Receiver`, the kind of an extension's `this`, which would make a dump of a closure parameter claim an extension.

### D4. Implicit `this` resolves against the innermost receiver only (autonomous)

Inside a receiver lambda, a bare name that is no local resolves through the lambda's `this` exactly as in an extension body (`extension-functions`, "The Receiver in Extension Bodies"): a field of the receiver's struct, then, when called, an intrinsic member or an extension on the receiver's type, then the module. The checker already reads the innermost `THIS` binding, so binding the lambda's `this` makes it the innermost receiver. An outer receiver, an enclosing extension's `this` or an outer lambda's, is not consulted: inside `fun User.Card() { Column { Text(name) } }` the name `name` is not `User`'s field; the body writes `val user = this` before the lambda, or takes `user: &User` as a parameter, which is how Compose code is written anyway.

This is Compose's behaviour for its scopes, whose `@LayoutScopeMarker` (a `@DslMarker`) hides an outer `RowScope` inside a `Column`'s content, applied to every receiver.

**Compile-time cost and Kotlin-likeness.** Constant per name, as today: one lookup in the innermost receiver. Kotlin consults every implicit receiver from the innermost out, except those hidden by a marker, which costs time in the nesting depth per unresolved name, quadratic in nested builders, and needs a marker annotation Prism lacks. The difference shows only for non-scope outer receivers, and the error says where the name would have been found.

Rejected:
- **Kotlin's receiver chain with a `@DslMarker`-like modifier**: an annotation system, and per-name work in the depth.
- **The chain without markers**: outer layout scopes would leak, so `Modifier.align` inside a `Box` in a `Column` would be ambiguous, which Compose forbids.
- **Labelled `this@Card`** now: useful, and compatible with this rule, but new syntax with its own scenarios; a follow-up.

The `E4001` help for a bare name that misses, when an enclosing outer receiver has a field or extension of that name, says so: "'name' is a field of the outer receiver 'User', which a lambda with a receiver hides; bind it before the lambda (`val user = this`)". It is computed only on that error path, by walking the receivers then.

### D5. Calling receiver function values: `f(r, a)`, `r.f(a)` and `f(a)` (autonomous: the resolution order)

A value `f` of type `T.(A) -> R` is called in Kotlin's three ways:
- **`f(r, a)`**: an ordinary call of a function value; `r` binds the receiver's parameter, borrowed implicitly for a shared receiver and with `&mut r` written for a mutable one, as any `&T` or `&mut T` parameter.
- **`r.f(a)`**: a method call. After members (a field `f`, the built-in `clone()` and `unbox()`, an intrinsic member), and before extensions, a call `value.f(...)` considers the innermost local or parameter named `f`: when its type is a receiver function type whose receiver type is the value's type, after the receiver's `&`, `&mut` and `Heap<T>` layers, the call is a call of that value with the value as its receiver, borrowed as a `fun` or `mut fun` receiver is (`references`, "Method Receivers Borrow Implicitly"). A local of another type is skipped, so extensions are found as before.
- **`f(a)`**: when a function value of receiver type is called with one argument fewer than its parameters and the innermost `this` has the receiver's type, `this` is passed as the receiver (reborrowed). This is how Compose's own scopes call their content (`fun RowScope.Cell(content: RowScope.() -> Unit) { content() }`). Without such a `this`, the call is `E4004`, saying that `f` takes a receiver of type `T` and how to pass it (`f(scope)` or `scope.f()`).

**Compile-time cost.** `r.f(a)` adds one lookup of `f` among locals to a method call that found no member, a hash lookup in the scoped symbol table. `f(a)` compares one argument count and one type. Kotlin's three forms.

Rejected: only `f(r, a)` (the form `ui-toolkit` writes, but not how Compose's sources read); extensions before local values in `r.f(a)` (a parameter named like an imported extension would be unreachable in method form, and Kotlin resolves locals first); implicit receivers from outer scopes for `f(a)` (D4).

### D6. `context(name: T)` declarations (autonomous: one parameter, its place, its borrow)

A function declaration may start with one context parameter, Kotlin 2.2's syntax: `context(scope: RowScope) public fun Modifier.weight(weight: Double, fill: Bool = true): Modifier`. It comes before every other modifier (visibility, `compose`, the receiver mode), names its parameter and gives its type, which follows the rules of D1's receivers and may not mention the declaration's own type parameters. It applies to any function with a body: an extension, a plain function, a `compose fun`. In the body and the defaults, the parameter is a `val` of type `&T`, readable like any parameter; it is not an implicit receiver, so bare names do not resolve through it (Kotlin 2.2's rule: context parameters are not receivers).

The context is passed as the function's first parameter, a shared borrow, before the receiver: `weight(scope, this, weight, fill)` in the typed tree, MIR and the backend (Kotlin's JVM convention also puts context parameters first). `compose-transform`'s change words then cover it as parameter 0, which it compares like any other (an empty `Copy` scope always compares equal).

`context` is a contextual keyword: a modifier only at a top-level declaration's start followed by `(`; elsewhere an identifier. A second context parameter, `context` on a struct, enum or `val`, `context()` without a parameter, and `context` on an `external` or `intrinsic` declaration are `E2001`.

**Compile-time cost and Kotlin-likeness.** Parsing is one token of lookahead; checking a declaration binds one more parameter. Kotlin's syntax, limited to one parameter.

Rejected:
- **`fun RowScope.weight(m: Modifier, w: Double)`** or an explicit scope argument: unlike Compose's `Modifier.weight(1f)`.
- **Kotlin's member extensions** (`weight` declared inside `RowScope`): Prism has no member functions (`extension-functions` rationale) and no interfaces.
- **Several context parameters**: with D7's innermost-only resolution, a second one could only come from an enclosing context function; nothing needs it yet. A follow-up.
- **Context parameters as implicit receivers** (Kotlin's withdrawn context receivers): two implicit receivers per position, which D4 avoids.
- **Passing the context by value**: a scope may one day hold state, and a shared borrow is what an extension's `fun` receiver gets.

### D7. A call's context: the innermost receiver lambda, or the enclosing function (autonomous)

The **context** of a position is the innermost enclosing receiver lambda's `this`, when the position is inside one in the current function (through ordinary closures in between); otherwise the enclosing function's extension receiver and its context parameter, either or both. A call of a function with a context parameter of type `T` passes, shared-borrowed, the first candidate whose type after references is exactly `T` (the receiver before the context parameter); when none matches, the call is `E4034` (D10).

So `Modifier.weight(1.0)` resolves in a row's content (`RowScope.() -> Unit`), in a `fun RowScope.Cell()` body and in a `context(scope: RowScope) fun Modifier.half()` body, but not in a `Box`'s content inside a row, whose innermost receiver is a `BoxScope`, as in Compose.

**Compile-time cost.** The checker keeps the current context (at most two locals) in its function and closure state, replaced on entering a receiver lambda or a function body and restored on exit: constant per lambda, and at most two type comparisons per context call.

Rejected: searching every enclosing receiver (D4); requiring an explicit context argument (`with(scope) { ... }` blocks): not Compose's surface.

### D8. Extensions may differ only by context (autonomous: per-layer resolution, no plain overloads)

The extension tables are keyed by receiver type constructor and name as today, and each key holds at most one context-free extension and at most one per context type. Two extensions of one file with the same receiver constructor, name and context (both none, or the same type) are `E4010`; a different context is not a duplicate.

A call `value.name(...)` resolves, layer by layer (the file's own, the imports, the prelude, as today): within the first layer that has an extension of that key, the one whose context type is that of a context candidate (D7) is chosen, else the context-free one, else the call is `E4034` naming the contexts that layer declares. Several imports with a matching extension of one key and context are `E4012`, as for any extension. Plain functions are not overloaded: two top-level functions of one name stay `E4010` whatever their contexts.

**Compile-time cost.** One map lookup more per extension call (the candidate's type, then none). The ABI keeps one entry per extension, keyed by context too (D9).

Rejected:
- **Choosing across layers** (an imported context match before the file's own context-free extension): the file's own declaration would no longer shadow imports, contrary to `extension-functions`.
- **Preferring the context-free extension**: then `ColumnScope`'s `weight` could never be called where a context-free `weight` exists.
- **Overloading plain functions by context**: the name scopes hold one value per name; nothing needs it yet.

### D9. Typed tree, MIR, borrows, ABI and symbols

- **Typed tree.** A receiver lambda's `parameters` start with its `ClosureReceiver` local (D3). A call of a receiver function value is a call of the value whose first argument is the receiver, a shared or mutable borrow as for an extension's receiver, or a reborrow of the innermost `this` for `f(a)`. A call of a function with a context starts its arguments with the context argument, a shared borrow of the candidate's referent marked as the context (`context` in the dump), then the receiver. An extension with a context is named by its module, receiver, name and context, dumped `<module>::<receiver>.<name>#context(<type>)`.
- **MIR, borrow checker, backend.** No new construct: the context argument leads a call's operands, before the receiver. Borrows follow the parameters' types.
- **Elision** (autonomous). A receiver function type elides as every function type does, by rules 1, 3 and 4 (`references`, "Function Types Elide by the Same Rules"): the receiver is an input borrow like any reference parameter, not rule 2's receiver. Rule 2 would make `T.(&A) -> &U` tie its result to the receiver only while `(&T, &A) -> &U` ties it to both, so the in-place conversion of D2 would be unsound. A context parameter is an input borrow like a parameter, so a context extension's rule 2 still ties a returned borrow to the receiver only.
- **ABI.** A structured function type records whether it has a receiver (its first parameter); an extension's entry records its context type, bound like any signature type; the hash covers both, and the order of exported symbols takes the context into account after the receiver and name, so reordering declarations never changes the hash.
- **Symbols.** An extension with a context is `<module>::<receiver>.<name>#context(<type>)`, the context type written as the ABI writes its type constructor; its default generators append `#default(<parameter>)` as usual. A plain function's symbol is unchanged, since it cannot be overloaded by context.

**Compile-time cost.** No pass is added; dumps print one more argument and parameter.

### D10. Diagnostics: a new `E4034`, and widened `E4001`, `E4004` and `E4010` (autonomous)

- **`E4034` `MissingContext`**: a call of a function with a context parameter whose context has no candidate of its type. "'weight' needs a context of type 'RowScope'", with the context the call has ("the innermost receiver here is 'BoxScope', the receiver of this lambda", "this position has no receiver"), the contexts the extension is declared for when there are several ("'weight' is declared for the contexts 'RowScope' and 'ColumnScope'"), and help: call it inside a lambda with a `RowScope` receiver, or declare the enclosing function `context(scope: RowScope)`. Naming a plain function with a context as a value is `E4034` too, since its value would need its context; an extension is no value anyway (`E4006`). The code is the next one after those `compose-transform` and `state-handles` reserve (`E4030` to `E4033`), so the packets landing in any order keep their codes.
- **`E4001`**: `this` outside an extension or a receiver lambda (message updated), and D4's help for a name found only on an outer receiver.
- **`E4004`**: a receiver function value called without a receiver of its type (D5).
- **`E4010`**: two extensions of one receiver constructor, name and context.
- **`E2001`**: the syntax errors of D1 and D6.

Rejected: reporting a missing context as `E4006` ("no extension `weight` on `Modifier`"), which hides that the extension exists and what it needs, and gives the plain-function case no code.

### D11. Formatting (autonomous: the context on its own line)

`prism fmt` prints receiver function types as written, with one space after `compose`, `mut` and `escaping` and none around the `.` (`compose mut Builder.(Int) -> Unit`), and a context parameter on its own line directly above the rest of its declaration (`context(scope: RowScope)` then `public fun Modifier.weight(...)`), like an annotation, keeping signatures within the line width. Comments above a declaration stay above its context line, which counts as the declaration's first line (so `library_tests.rs`'s rule of a `//` comment directly above every declaration line means above the context line). The meaning check compares receivers and contexts.

Rejected: the context on the declaration's line (`ui-toolkit` D22's sketch), which makes most scoped modifiers' signatures break their parameter lists.

### D12. Composable receiver lambdas

`compose ColumnScope.() -> Unit` is a composable function type with a receiver; a lambda checked against it is checked in a composable context and has its `this`, and composable and non-composable receiver types do not convert into each other. Nothing in `compose-transform`'s plan changes: a composable closure gets the composer after its declared parameters, and its receiver is parameter 0 of its change words, as an extension composable's receiver is ("the receiver first"). If this change is archived first, `compose-transform`'s apply finds receiver closures as closures with one more parameter; if `compose-transform` is archived first, this change's apply adds a composable receiver lambda to its fixtures.

### D13. Compile-time shapes

`crates/prism-cli/tests/compile_timing.rs` gains three shapes, held to roughly linear growth under `PRISM_ASSERT_TIMING=1`:
- `receiver-lambdas`: `n` receiver lambdas in sequence, each reading a field and calling an extension through its `this` and calling a context extension;
- `nested-receiver-lambdas`: receiver lambdas nested 16 to 128 deep, alternating two scope types, each level calling a context extension of its scope and the innermost reading a captured local of the outermost, at most x2.5 per doubling of the depth as the existing depth shapes;
- `context-overloads`: `n` scope types, an extension of one name and receiver per scope, and `n` calls each in its own scope's lambda.

Measured on a release build (2026-10-19, `PRISM_ASSERT_TIMING=1`, passing): `receiver-lambdas` grows at most x4.2 per x4 (MIR; type checking 3.2, 13.2 and 38.0 ms at 256, 1024 and 4096 lambdas); `nested-receiver-lambdas` grows x1.97 (type), x2.15 (MIR), x2.03 (borrow), x2.08 (lower) and x1.97 (Cranelift) per doubling of the depth on average; `context-overloads` grows at most x4.7 per x4 (MIR; type checking 3.0, 13.1 and 58.2 ms), so resolving among 4096 extensions of one name costs a lookup per call, not a scan. The extension tables index each name's extensions by context type to keep it so.

### D14. Spec placement

No new capability: syntax to `syntax`; the type, its conversions and calls of its values to `type-system`; `this` in receiver lambdas, method-call resolution, context parameters and their overloading to `extension-functions`, where extension bodies' `this` lives; borrows and elision to `references`; the tree to `typed-tree`; operand order to `mir`; the ABI to `compilation-and-abi`; symbols to `code-generation`; printing to `formatting`; running to `execution`. `diagnostics/spec.md` is edited directly at apply time, as every change does. A `receiver-lambdas` capability was rejected because it would split each subject across two specs.

## Risks / Trade-offs

- [The innermost-receiver rule rejects Kotlin code that reads an outer receiver's field inside a receiver lambda] → `E4001`'s help names the outer receiver and the fix; labelled `this@name` is a follow-up; Compose code rarely has non-scope receivers around content lambdas.
- [`r.f(a)` now finds a local before an extension of the same name] → only when the local has a receiver function type of a matching receiver, which no existing program has.
- [Receiver and plain function types convert in both directions, so `List<ColumnScope.() -> Unit>` and `List<(&ColumnScope) -> Unit>` mix] → they share a representation, and the conversion is the one Kotlin allows.
- [Extensions differing by context need new symbols] → only extensions with a context get the suffix; every existing symbol is unchanged.
- [`ui-toolkit` D22 sketches `context(...)` on the declaration's line] → the formatter moves it (D11); `ui-toolkit`'s task 1.1 compares the names this change leaves.

## Migration Plan

No external users and no existing program changes meaning: no function type has a receiver, no declaration a context, and `context` stays an identifier everywhere it was one. The `E4001` message for `this` changes; fixtures pinning it are updated. Rollback is a revert.

## Open Questions

- The exact wording of `E4034`'s and `E4001`'s helps can be tuned during implementation.

## Implementation Notes

- **Changed scenario.** "Converting between receiver and plain function types" passed `c` to `plain(c)` and then converted `c` again, which uses a moved value (`E5001`): function values are not `Copy`. It now converts a second closure `d` to the mutable receiver type, which keeps its intent (each direction of D2 converts) and passes `prism check`.
- **Glue names.** Drop and clone glue are keyed by type, and a receiver function type and its plain form are distinct types, so glue names write the receiver as source does (`main::Scope.() -> Unit`); otherwise a unit needing the glue of both would define one symbol twice. The contract hash, which describes representations, ignores the receiver. The MIR codec gives a function type with a receiver its own tag, so plain function types encode as before.
- **`LocalKind::ClosureReceiver` lands with 3.1**, since binding a receiver lambda's `this` needs it; task 5.1 then only checks its dump and validation.
- **A new `E4001` variant, `OuterReceiverName`.** D4's help for a name found only on an outer receiver is its own variant rather than a help field on `UnresolvedSymbol`, whose message says that only the innermost receiver is implicit; the help names the receiver and the binding to write. It is computed only when a name does not resolve, by walking the `this` bindings the innermost one shadows.
- **Receiver value calls are not receiver calls in the tree.** A call of a receiver function value keeps `has_receiver` false, which marks an extension's receiver (MIR borrows that one after the other arguments, and elision rule 2 applies to it): the receiver is an ordinary first argument, evaluated first, borrowed with the receiver origin in `r.f(a)`, as an argument in `f(r, a)`, and reborrowed from `this` in `f(a)`. The `f(a)` form applies to a function value called with exactly one positional argument fewer than its parameters.
- **Context plumbing lands with group 4.** Running a context call needs the tree, MIR and symbols to know the context, so `DeclarationPath::context`, `Call::has_context`, `FunctionItem::context` and the `#context(...)` symbol suffix land with 4.1; tasks 5.1 to 5.3 then pin them with snapshots. MIR finds an extension's receiver after the context argument, a default generator's parameter index skips both, and a default value reads the context too. The context argument records the parameter name `context`, since the ABI does not record the declared name.
- **A context's candidates are bindings.** The checker keeps the current context as the bindings of its candidates (the lambda's `this`, or the function's `this` and context parameter), so a local that shadows a context parameter's name never becomes the context argument.
- **The plain-function value case.** Naming a plain context function outside a call is `E4034` with the message that it can only be called; whether it is a callee is the flag generic callees already use.
- **`E4034`'s catalog row** records that `E4030` to `E4033` are reserved by `compose-transform` and `state-handles`.
- **Changed scenario.** "Symbols of extensions differing by context" wrote the receiver and the context by their bare names; symbols write a struct receiver as the ABI writes its type constructor (`lib/layout.Modifier`), so the scenario now names `lib/layout::lib/layout.Modifier.weight#context(lib/layout.RowScope)` and its siblings. Dumps of the typed tree and MIR keep the short form (`main::Modifier.weight#context(RowScope)`).
- **Elision of a context extension.** The borrow checker's signatures record whether a function has a context, so rule 2 ties a context extension's result to its receiver, the second parameter, and the context counts as an input borrow like any parameter.
