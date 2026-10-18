# Proposal: receiver-lambdas

## Why

Jetpack Compose types layout content as lambdas with a receiver, `@Composable ColumnScope.() -> Unit`, and offers `Modifier.weight(1f)` and `Modifier.align(...)` only inside the matching row, column or box content. The `ui-toolkit` revision of 2026-10-19 (its D22, by user direction to follow Compose's APIs) needs both, and Prism has neither: function types have no receiver, a lambda has no implicit `this`, and an extension cannot require a scope. Kotlin builder DSLs (`buildList { add(x) }`, HTML-like builders) need the same feature. This change adds it as a small, independent language change, placed before `ui-toolkit` in `ROADMAP.md`.

## What Changes

- **Function types with a receiver.** `T.(A, B) -> R` is a function type whose first parameter is its receiver, borrowed shared (`&T`), and `mut T.(A) -> R` borrows it mutably (`&mut T`), as `fun` and `mut fun` extensions do. `compose` and `escaping` apply as to any function type: `content: compose ColumnScope.() -> Unit`, `onEvent: escaping mut Log.(String) -> Unit`. The receiver is a named type, with type arguments or a type parameter of the declaration. A receiver function type is its own static type, but has the representation of `(&T, A) -> R`, and the two convert into each other in place, as Kotlin lets `T.() -> R` and `(T) -> R` be used for each other.
- **Lambdas with a receiver.** A lambda checked against a receiver function type has an implicit `this`, the receiver, bound like a `val` before its declared parameters. Inside it, bare names resolve through `this` exactly as in an extension body: fields, then extensions on the receiver's type, then the module. Only the innermost receiver is consulted: an outer extension's or lambda's `this` is not, as Compose's `@LayoutScopeMarker` makes outer layout scopes unavailable. A header declares only the other parameters, so a `T.(A) -> R` lambda without a header has the implicit `it`.
- **Calling receiver function values** in Kotlin's three ways: `content(scope)`, `scope.content()`, and `content()` where the innermost `this` has the receiver's type. In a call `value.name(...)`, a local or parameter `name` of a receiver function type whose receiver is the value's type is considered after members and before extensions.
- **Context parameters**, Kotlin 2.2's `context(scope: RowScope) fun Modifier.weight(weight: Double): Modifier`: at most one, written before the declaration's modifiers, on any function with a body. The function is called only where the call's context has that type: the innermost receiver lambda's `this`, or, outside any receiver lambda, the enclosing function's receiver or context parameter. The context is passed as a leading shared-borrow argument and is a parameter in the body. **Extensions may share a name and receiver when their contexts differ**, so `RowScope`'s and `ColumnScope`'s `weight` coexist, and a call picks the one its context matches.
- **Diagnostics.** A call of a function whose context is missing is a new error, `E4034` (`MissingContext`), naming the context the function needs and the one the call has. `this` outside an extension or a receiver lambda stays `E4001`, with its message updated. A receiver function value called without a receiver of its type is `E4004`. New syntax errors are `E2001`.
- **Typed tree, MIR and backend.** The receiver of a lambda is a closure-parameter local of a new kind, the lambda's first parameter, so MIR, the borrow checker and the backend see a closure with one more parameter. A context argument leads a call's arguments, before the receiver. Extensions with a context have their own declaration paths and symbols (`<module>::Modifier.weight#context(RowScope)`).
- **ABI and formatter.** The ABI records a function type's receiver and an extension's context, both covered by the hash. `prism fmt` prints receiver function types as written and a context on its own line above its declaration.
- **Not in scope:** `consuming` receivers in function types; several context parameters; context parameters on `external` or `intrinsic` declarations; plain functions overloaded by context; labelled `this@name` for outer receivers; Kotlin's `@DslMarker` (the innermost-only rule stands in for it); extensions on a bare type parameter (`fun <T> T.apply(...)`).

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `syntax`: function types with a receiver; `context(...)` declarations; `context` is a contextual keyword.
- `type-system`: receiver function types (identity, conversion, display), lambdas with a receiver, and calling receiver function values.
- `extension-functions`: `this` in receiver lambdas and the innermost-receiver rule; receiver function values in method-call resolution; context parameters, their resolution and `E4034`; extensions differing only by context.
- `references`: how receivers of function values and context arguments are borrowed, and how receiver function types elide.
- `typed-tree`: the receiver local of a lambda, normalized receiver-value calls, context arguments and the paths of context extensions.
- `mir`: context arguments lead a call's operands.
- `compilation-and-abi`: receiver function types and context parameters in the ABI.
- `code-generation`: symbols of extensions with a context.
- `formatting`: printing receiver function types and context parameters.
- `execution`: receiver lambdas and context extensions run, in a hand-written layout DSL and a builder, under `--check-allocations`.

## Impact

- `crates/prism-lexer`, `crates/prism-parser`, `crates/prism-ast`: `TypeKind::Function` gains a receiver; `FunctionDecl` gains a context parameter; `context` and `mut` before a type are parsed.
- `crates/prism-typed-tree`: `Type::Fn` gains a receiver flag (its first parameter is the receiver); `LocalKind::ClosureReceiver`; context arguments; dumps.
- `crates/prism-typechecker`: receiver lambdas, receiver-value calls, the context of a position, context-keyed extension tables, `E4034`.
- `crates/prism-abi`: the receiver in structured function types, the context in extension entries, symbol order and hashing.
- `crates/prism-borrowck`, `crates/prism-mir`, `crates/prism-codegen`: no new behaviour; the context-extension symbol suffix in codegen.
- `crates/prism-formatter`: receiver types and context lines.
- `crates/prism-cli`: fixtures under `tests/fixtures/{syntax,type-system,extension-functions,references,typed-tree,mir,compilation-and-abi,code-generation,execution}/`, executable ones under `--check-allocations`; formatter goldens; compile-timing shapes.
- `openspec/specs/diagnostics/spec.md`: the new `E4034` row, and the `E4001`, `E4004` and `E4010` rows widened.
- Unblocks `ui-toolkit` D22 (`ColumnScope.() -> Unit` content, scoped `weight` and `align`), whose scenario "A scoped modifier outside its scope" expects `E4034`.
