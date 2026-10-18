# Proposal: implicit-it

## Why

A closure passed where a function of one parameter is expected must name its parameter today (`xs.forEach { x -> println(x) }`), and `{ println(it) }` is `E4004`, "closure expected 1 parameters, but has 0". Kotlin and Jetpack Compose code leans on the implicit `it` everywhere: `count.update { it + 1 }`, `todos.edit { it.add(Todo("x")) }`, `items.forEach { Text(it.name) }`, `onValueChange = { text = it }`. The `state-handles` change writes its handlers with an explicit parameter only until this exists. The item is deferred by `harden-parser-grammar` in `ROADMAP.md` ("Implicit `it` parameter for single-parameter closures") and lands before the compose changes so their examples read as Kotlin.

## What Changes

- **A closure without a parameter header has an implicit parameter `it` exactly when its expected type is a function type with one parameter** (Kotlin's rule). `it` has that parameter's type and is bound like a declared closure parameter: a `val` binding, `escaping` when the parameter is, a reference for a `(&T) -> R` parameter, read through it with the usual copy-out and implicit borrows. Against a function type with no parameters a header-less closure still has none; with no expected function type (`val f = { it }`) it has none either. The decision is local to the closure: the expected type is known when the closure is checked, and nothing is inferred from the body, re-checked or backtracked.
- **`it` is an ordinary identifier, not a keyword.** `val it = 1`, a parameter named `it` and `{ it -> it + 1 }` stay valid. The implicit `it` is declared in the closure's own scope, so it shadows any outer `it`, including the `it` of an enclosing closure: in `xs.forEach { ys.forEach { println(it) } }` the inner `it` is the element of `ys`. A closure without its own `it` (one with zero expected parameters) sees the enclosing closure's `it`, as in Kotlin.
- **`{ -> body }` declares zero parameters explicitly**, so it never gets an implicit `it`, and the syntax tree records whether a closure has a header. **BREAKING (formatter):** `prism fmt` keeps an empty header `{ -> 1 }` instead of printing `{ 1 }`, since the two now mean different things against a one-parameter type. The formatter never adds or removes `it` or a header.
- **Diagnostics.** A use of `it` where no closure binds it is `E4001`, with help saying why this closure has no `it` (its expected type has no parameters, it has no expected function type, or it declares its parameters) or, outside any closure, what `it` names. A header-less closure whose expected function type has two or more parameters stays `E4004`, now saying "closure expected 2 parameters ('Int', 'String'), but declares none", with help writing the header to add and saying that only a closure expecting one parameter has an implicit `it`. Not using `it` is fine. No new code.
- **Typed tree:** the implicit `it` is an ordinary closure-parameter local named `it`, of a new local kind "implicit closure parameter", declared at the closure's `{`. **MIR, borrow checker and backend:** no change; the closure lowers with one parameter, as if it had been written `{ it -> ... }`.
- **Not in scope:** destructuring closure parameters (`{ (a, b) -> }`), parameter type annotations in closure headers (`{ x: Int -> }`), `_` for unused closure parameters, and a warning for an `it` that shadows an outer `it` (Prism has no warnings yet).

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `syntax`: "Braces Are Closures in Expression Position" says a closure without a header declares no parameters and distinguishes the empty header `{ -> }`; `it` is not a keyword.
- `type-system`: the implicit `it` (when a closure has one, its type and binding, shadowing), its diagnostics, and the arity mismatch of a header-less closure.
- `typed-tree`: the implicit parameter is a closure-parameter local named `it`.
- `formatting`: an empty closure header is kept.
- `references`: a closure's implicit `it` borrows and escapes like a declared parameter.
- `execution`: closures using `it` run like closures declaring it, in list helpers, generic higher-order functions, through references and nested.

## Impact

- `crates/prism-ast`: `ExprKind::Closure` records the `->` of its header (`arrow: Option<Span>`); its `Display` prints `{ -> ... }` for an empty header.
- `crates/prism-parser`: `parse_closure` stores the arrow.
- `crates/prism-abi`: the span-free body hash includes whether a closure has a header (a one-line change).
- `crates/prism-typechecker`: `check_closure` binds the implicit `it`; a new `E4001` kind for an unbound `it` with its help; a new `E4004` kind for a closure's parameter count with the expected types and the header to write.
- `crates/prism-typed-tree`: `LocalKind::ImplicitClosureParameter`, its dump label and its validation.
- `crates/prism-formatter`: prints and verifies the empty header.
- `crates/prism-cli`: renders the new helps; fixtures under `tests/fixtures/{syntax,type-system,typed-tree,references,execution}/`, executable ones under `--check-allocations`; compile-timing shapes for many and deeply nested implicit-`it` closures.
- `openspec/specs/diagnostics/spec.md`: the `E4001` and `E4004` rows mention `it` and closure parameter counts.
- `ROADMAP.md`, `docs/decisions-for-review.md`: the decisions below, and on archive the deferred items.
