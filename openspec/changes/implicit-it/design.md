# Design: implicit-it

## Context

See `proposal.md` for motivation. This change takes the ROADMAP item "Implicit `it` parameter for single-parameter closures (deferred by `harden-parser-grammar`)", planned autonomously on 2026-10-18 as part of the run toward Milestone 2; every decision the user might make differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

Current state:

- **Parser** (`parse_closure`): `is_closure_with_arrow_ahead` scans `{ name, name ->` and decides whether a closure has a header. The AST is `ExprKind::Closure { params, body }`, so `{ -> 1 }` and `{ 1 }` are the same tree: both have zero parameters.
- **Type checker.** Every closure checked against a function type goes through `check_closure` (from `check_expr`, from a reference parameter's referent in `check_argument`, and from `check_deferred_closure` for a generic call whose closure parameter types wait for the other arguments). It reports a parameter-count mismatch as `CallSignatureMismatch` (`E4004`, "closure expected 1 parameters, but has 0", generic help) and checks nothing else. A closure with no expected function type goes through `infer_expr`, where each declared parameter is `E4013`. Parameters are bound with `bind_local` as `Mutability::Val`, `LocalKind::ClosureParameter`, with the expected type's `escaping` flag, in the scope `enter_closure` opens. An unknown name is `UnresolvedSymbol` (`E4001`) from `resolve_ident`, with special cases for built-ins and unimported types. Shadowing an outer binding in an inner scope is allowed (`fun f(it: Int) { val it = 2 }` is accepted), and a duplicate in one scope is `E4010`.
- **Typed tree.** `ExpressionKind::Closure { parameters: Vec<LocalId>, .. }`; each `Local` has a name, type, `LocalKind`, `escaping` flag and span. Captures are recorded at use (D11 of `typed-tree`), so a closure's own parameters are never captures, and an enclosing closure's parameter used in a nested closure is a capture of the nested one.
- **MIR, borrow checker, backend** read closure parameters from the typed tree only.
- **Formatter** (`printer/expressions.rs`, `closure`): prints a header only when `params` is non-empty, so `{ -> 1 }` is printed `{ 1 }` today. `verify.rs` compares span-erased trees.
- **ABI** (`hash_expression_span_free`): a closure hashes its parameter count and names, then its body.

## Goals / Non-Goals

**Goals:**
- Kotlin's implicit `it`, decided at the closure from what is already known when it is checked: no inference from the body, no second check, no backtracking.
- No change after the type checker: the closure the typed tree records is the one `{ it -> ... }` gives.
- Diagnostics that say why a closure has no `it`, and a better arity mismatch for header-less closures.
- A fixture for every scenario, executable ones under `--check-allocations`.

**Non-Goals:**
- Destructuring parameters (`{ (a, b) -> }`), parameter annotations in headers (`{ x: Int -> }`), `_` for unused parameters.
- Giving `it` to a closure with no expected function type by inferring its parameter from the body (D1).
- A shadowing warning for nested `it`s (Prism has no warnings yet; a lint later).
- Kotlin's labelled `return@forEach` or `this@...`.

## Decisions

### D1. A header-less closure has `it` exactly when its expected function type has one parameter

In `check_closure`, a closure without a header whose expected function type has exactly one parameter binds one parameter named `it`; every other case behaves as today. This is Kotlin's rule (a lambda without a parameter list gets `it` when its expected type has one parameter). It covers every expected-type source the checker already has: a parameter type (including a trailing lambda), a reference parameter's referent, an annotation, a return type, a deferred closure of a generic call once its parameter types are solved (D8 of `generics`), a struct field or enum payload, a named or default argument, and a nullable function type (`val f: ((Int) -> Int)? = { it + 1 }`), against whose function type a closure is checked before it is made nullable (found in review: before, a closure there was inferred without an expected type, so `it` and declared parameters were `E4001` and `E4013`).

**Compile-time cost and Kotlin-likeness.** One comparison of the expected parameter count per closure, at the point where `check_closure` already compares it; no new pass, no lookahead into the body, no state beyond what the closure scope already holds. Kotlin's surface exactly, except where Kotlin infers more (below).

Rejected:
- **Inferring `it` from the body when there is no expected type** (`val f = { it + 1 }`), which Kotlin also rejects: it needs constraint solving across the body (`typechecker-soundness` D6 rejected it for declared parameters for the same reason).
- **Deciding from the body** (a header-less closure has `it` when its body mentions `it`, as Swift's `$0` effectively does): then `{ ... }` against `(Int) -> Unit` without `it` in the body would be an arity error, Kotlin accepts it, and every closure body would need a scan before checking.
- **Re-checking a closure body as one-parameter after a zero-parameter attempt fails**: speculative work per closure, and quadratic on nesting.

### D2. `{ -> body }` declares zero parameters, so the tree records the header (autonomous)

`ExprKind::Closure` gains `arrow: Option<Span>`, the `->` of the header, `None` without one. `{ -> 1 }` against `(Int) -> Int` is the arity mismatch (D6), as in Kotlin, and against `() -> Int` it is the same as `{ 1 }`. The parser already decides whether a header exists; it now keeps the arrow. The formatter prints a header whenever `arrow` is set (`{ -> 1 }`, `{ -> }`), the meaning check erases the arrow's span like any other, and the span-free body hash includes whether a closure has a header, since `{ -> x }` and `{ x }` mean different things in a generic body.

A span rather than a flag: the arrow's position serves the formatter's comment placement and a future LSP, and the field's presence is the flag. **BREAKING (formatter only):** `prism fmt` stops rewriting `{ -> 1 }` to `{ 1 }`. No fixture or example writes an empty header.

Rejected: treating `{ -> body }` as `{ body }` (no AST change, and the formatter could keep normalizing it), which gives `{ -> it }` an `it` against `(Int) -> Int` and leaves no way to say "this closure takes nothing"; `params: Option<Vec<ClosureParameter>>`, which renames the meaning of an existing field and makes every consumer unwrap it.

### D3. `it` is an ordinary identifier that the implicit parameter shadows (autonomous)

`it` is not a keyword, contextual or otherwise: no token, no grammar rule, no reserved name. `val it`, `fun it`, a parameter or field `it`, and `{ it -> ... }` stay valid. The implicit `it` is bound in the closure's own scope, which `enter_closure` already opens, so it shadows any outer `it` by the ordinary scoping rule: a local, a parameter, a top-level declaration, and the `it` of an enclosing closure. A closure without its own `it` (zero expected parameters) sees the enclosing one, so `xs.forEach { run { println(it) } }` prints the element, as in Kotlin.

**Compile-time cost.** None: one more local in the closure's scope, found by the ordinary lookup.

Rejected: a contextual keyword (`it` is resolved specially inside closures only), which needs a rule for every place a name may appear and buys nothing, since lookup is already lexical; rejecting a nested implicit `it` that shadows another (Kotlin only warns, and the shape `xs.forEach { ys.forEach { ... it ... } }` is common in Compose list code); making the outer `it` unreachable inside every nested closure (Swift-like `$0` per closure), which breaks Kotlin's `run { it }` reading the enclosing parameter.

### D4. `it` is bound like a declared parameter

`it` is a `val` binding of exactly the expected parameter's type, carrying its `escaping` flag, so everything downstream follows from the rules for `{ it -> ... }`: for `(&T) -> R` it is a `&T`, read through with copy-out and passed on as a reborrow; for `(&mut T) -> R` it writes through (`todos.edit { it.add(x) }`); for an owned `String` it is dropped at the end of the call unless moved; a non-`escaping` function-typed `it` cannot be kept (`E5007`); a reference `it` follows the closure type's elision rules. A deferred generic closure whose parameter type is still unsolved is `E4013` before any `it` is bound, as for declared parameters.

### D5. The typed tree records an implicit closure parameter local (autonomous)

The closure's `parameters` hold one local named `it`, of a new kind `LocalKind::ImplicitClosureParameter`, dumped as `(implicit closure parameter)`, whose span is the closure's `{`. Uses of `it` are ordinary `Local` reads. The validator treats the kind like `ClosureParameter`. MIR, the borrow checker and the backend do not look at the kind, so they need no change.

Rejected: a plain `ClosureParameter` (an LSP and diagnostics could not tell a written `it` from an implicit one, and "go to definition" would land on a `{` without explanation); desugaring in the parser to `{ it -> ... }`, which the parser cannot do because it does not know the expected type, and which would make the formatter print a header the user never wrote.

### D6. Diagnostics: `E4001` for an unbound `it`, `E4004` for a header-less closure expecting several parameters (autonomous)

- **Unbound `it`** (`TypeError::UnresolvedImplicitParameter`, `E4001`): reported where `resolve_ident` would report `UnresolvedSymbol` for the name `it`. Its message stays "unresolved symbol 'it'", and its help depends on the innermost enclosing closure, which `enter_closure` records (a small enum saved and restored with the closure state): its expected type has no parameters ("this closure's type '() -> Int' has no parameters, so it has no implicit 'it'"); it has no expected function type ("only a closure passed where a function of one parameter is expected has an implicit 'it'"); it declares parameters ("this closure declares its parameters ('x'), so it has no implicit 'it'"); or there is no closure ("'it' is the implicit parameter of a closure passed where a function of one parameter is expected"). The help is computed only on that error path.
- **Parameter count** (`TypeError::ClosureParameterCount`, `E4004`, replacing the closure use of `CallSignatureMismatch`): "closure expected 2 parameters ('Int', 'String'), but declares none" (or "but declares 1"), with help: for a header-less closure expecting two or more, "declare them in a header, as in '{ a, b -> ... }'; only a closure expecting one parameter has an implicit 'it'"; for a closure declaring the wrong number, "declare 2 parameters, as in '{ a, b -> ... }'", or "remove the header" when none are expected. The body is still not checked, as today.

The brief for this change suggested `E4001` at a use of `it` in a closure expecting two or more parameters. Rejected: the arity mismatch is the mistake, and it is reported whether or not the body uses `it`; reporting both would give one mistake two codes, and checking the body of a closure whose parameters are unknown would bind them to the error type for little gain. No new codes: both are existing kinds of error (an unresolved name, a closure that does not fit its parameter), so `diagnostics/spec.md` only widens the `E4001` and `E4004` rows.

### D7. Spec placement

No new capability: the header and the identifier status of `it` go to `syntax`, binding and diagnostics to `type-system`, the local kind to `typed-tree`, the empty header to `formatting`, borrowing and escapes to `references`, and runtime behaviour to `execution`. An `implicit-it` capability was rejected because it would split each subject across two specs.

### D8. Compile-time shapes

`compile_timing.rs` gains two shapes: `n` sequential header-less closures using `it` passed to a one-parameter function, and `n` closures nested `n` deep, each with its own `it` and reading the enclosing one through a captured `val`. Both must stay roughly linear, as the existing `many_closures` and `nested_closures` shapes are; nothing in D1 to D6 adds work per closure beyond a constant.

Measured while implementing (release build): `implicit-parameter-closures` grows x4.2 per x4 in type checking (7.8 ms at 4096 closures, as `many-closures`' 7.2 ms). `nested-implicit-parameter-closures` grows about x4 per doubling of the depth in type checking and in lowering (1.2 ms type and 2.2 ms lower at depth 120), and so does the same shape written with declared parameters (`apply { x -> ... }`), so the growth is not this change's: checking a call whose argument is a closure that itself ends in such a call costs time in the size of the argument somewhere in call checking and lowering, which the existing `nested-closures` shape (closures with no expected type, not passed as arguments) does not exercise. It is a ROADMAP follow-up; depths that matter in practice (under 10) cost microseconds.

Fixed in review (plain fix "Make nested trailing-lambda calls compile in linear time"): the type checker's scope stack hashed a name once per enclosing scope at every lookup, and the backend rebuilt a closure's symbol from all its enclosing closures' at every use. A scoped symbol table and symbols built once per file make both linear in the depth; `compile_timing.rs` gains five depth shapes (16 to 128 nested trailing-lambda calls, plain, with declared parameters, generic, generic with `it`, and a compose UI) held to at most x2.5 per doubling of the depth under `PRISM_ASSERT_TIMING=1`.

## Risks / Trade-offs

- [`it` silently refers to an outer `it` in a closure expecting zero parameters, where the author meant the closure's own] → this is Kotlin's behaviour; the type usually differs, and a lint can flag it once warnings exist.
- [Formatter output changes for `{ -> body }`] → no fixture or example writes it, and the formatter's meaning check now compares headers.
- [The arity message changes from "closure expected 1 parameters, but has 0"] → fixtures pinning the old text are updated; only the text changes, not the code.
- [`E5007` for an `it` that cannot escape helps "declare it as 'it: escaping () -> Unit'", which a closure parameter cannot write] → the same help is given today for a declared closure parameter (`{ g -> keepIt(g) }`); a better help (make the function type's parameter `escaping`) belongs to the borrow checker and is a follow-up.

## Migration Plan

No external users. In-repo, fixtures with header-less closures expecting one parameter (today `E4004`) are checked and updated; examples may switch to `it` where it reads better. Rollback is a revert.

## Open Questions

- The exact wording of the help lines can be tuned during implementation.
