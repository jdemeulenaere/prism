# Design: vararg

## Context

See `proposal.md` for motivation. This change is part **L4** of agreed item **A4** of the post–Milestone 2 review (`docs/milestone-2-review-notes.md`, "Agreed changes": "Language additions so the rest can be Prism: ... (L4) `vararg`"), item 9 of "After Milestone 2: the review round" in `ROADMAP.md` ("`vararg` (A4, L4): variadic parameters, so `listOf` is ordinary code"). The user is away: every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`; those taken in the review are **(user direction 2026-10-20)**. Compile speed is a primary criterion (AGENTS.md §5), so each decision states its compile-time cost.

**Baseline.** Proposed on top of `8c0a598` ("Implement owned-state: syntax, ABI and formatting"), with `owned-state` (item 4) being implemented and `strong-skipping` (item 5), `explicit-library` (item 6), `operator-conventions` (item 7) and `drop-and-clone-hooks` (item 8) proposed, all landing before this change. This packet is written for the state after them: no prelude, so `listOf` is used after `import /prism/collections` (explicit-library D1); the library under `library/prism/`; `E4035` and `E4036` taken (operator-conventions, drop-and-clone-hooks). Three of its blocks are written on another packet's version:
- `syntax` "Contextual Keyword Identifier Permissibility" on `owned-state`'s (which adds `attached` and `capture`);
- `standard-library` "The Collections Module Declares Lists and Their Helpers" on `explicit-library`'s ADDED block (the main spec's "The Collections Module" is removed by that packet);
- `collections` "Lists Are Built With listOf" and `type-system` "Trailing Lambdas Bind to the Last Parameter" on the main specs, which no open packet modifies.

The first task re-bases every block on the main specs once those packets are archived.

Current state (main specs and code):

- **`listOf` is the one variadic built-in** (`collections` D4). `library/prism/collections.prism` (today `crates/prism-library/prism/collections.prism`) declares `public intrinsic fun <T> listOf(element: T): List<T>`; the catalog (`crates/prism-typechecker/src/intrinsics.rs`) lists it as rule-typed, and `infer_call_expr` routes a call of it to `infer_list_construction` (`crates/prism-typechecker/src/checker.rs`), which builds a `GenericSignature` of `n` parameters of the element type `T` and checks the arguments with `check_generic_call_arguments`, so written type arguments, the expected type, joins (`listOf(1, null)` is `List<Int?>`), `E4004` and `E4013` come from generic call checking. The node is a `Call` of `Callee::Builtin(BuiltinFunction::ListOf)` whose declared parameters are the type parameter `T`, so closures given to it are stored values owning their captures (`collections`, "Elements Are Stored Values"). Named arguments are `E4004`. A trailing lambda is its last element (`collections` D22, scenario `listOf({ 1 }) { 2 }`).
- **MIR and code generation**: a `Call` terminator of `builtin listOf<T>` (`_1 = call builtin listOf<Int>(const 1, const 2)`); `crates/prism-codegen/src/lower.rs` evaluates the operands, allocates a buffer of exactly `n` elements once through `prism_rt_list_with_capacity` (none for `n = 0`) and moves each in.
- **Calls** (`check_call_arguments_with`): positional arguments fill parameters left to right (also after named ones), named ones by name, the trailing lambda the last parameter; parameter names and defaults are properties of the declaration (`SymbolKind::Function { param_names, defaults, .. }`), absent for function values, which therefore take neither.
- **`*` at the start of an expression** is `E2001` unless it starts an assignment target (`syntax`, "Writing Through a Reference"): `g(*r)` is rejected today.
- **`print`, `println`** (one argument with a text form, `println` also none) and **`remember`** (up to three keys of their own types) are the other rule-typed intrinsics.

## Goals / Non-Goals

**Goals:**
- Kotlin's `vararg` surface: declaration, calls, named parameters after it, spread.
- `listOf` written in Prism with no special case left in the type checker; the compiler keeps only the list construction, which a Rust-backed `List` needs.
- No new inference and no new analysis: typing a vararg call costs what typing `listOf` costs today, linear in its arguments.
- Zero-GC ownership with no copy: the caller builds the list once, the callee owns it.

**Non-Goals:**
- Heterogeneous varargs (`Any?`, variadic generics): `print`, `println`, `remember` and `key` keep their rules (D11).
- Spreads mixed with elements, borrowed spreads (D5).
- `vararg` in struct constructors and function types (D7, D9).
- `mutableListOf`, `setOf`, `arrayOf` (one list type, `collections` D3; no sets or arrays yet).

## Decisions

### D1. Kotlin's `vararg` modifier on function parameters (user direction 2026-10-20 for `vararg`; the syntax autonomous)

The review agreed "(L4) `vararg`". The syntax is Kotlin's: `vararg` before the parameter's name, the written type being the element type: `fun <T> listOf(vararg elements: T): List<T>`, `fun Builder.items(vararg names: String, separator: String = ", ")`. `vararg` is a contextual modifier, recognized only at the start of a parameter of a `fun` declaration and followed by the parameter's name; elsewhere it is a name (`fun vararg(vararg: Int)`, `fun f(vararg vararg: Int)`), so no program loses an identifier.

**Compile-time cost.** One token test per parameter.

Rejected:
- **Swift's `xs: Int...`** or **Rust-style macros**: not Kotlin.
- **A `List<T>` parameter with a call-site literal (`sum([1, 2, 3])`)**: Kotlin has no collection literal, and `listOf` itself would remain a special form.

### D2. Inside the body, the parameter is an owned `List<T>` built by the caller (autonomous)

A `vararg xs: T` parameter has type `List<T>` in the body, an ordinary by-value parameter: the caller builds the list from the written elements, moving or copying each in (one allocation of exactly their number, none for zero), and moves the list into the call; the callee owns it, so it may read it, iterate it, change it after `var list = xs`, move elements out, keep it or return it. `listOf` is therefore `= elements`, with no copy. Kotlin's vararg is likewise an array the callee owns (`Array<out T>`), allocated per call. Since `List<T>` needs a borrow-free `T`, a `vararg` whose element type holds a reference outside a function type is `E4022` at the declaration, with help naming the referent type (`vararg names: String`).

**Compile-time cost.** None: the body sees an ordinary parameter.

Rejected:
- **A borrowed `&List<T>`** (the callee reads only): the caller would still allocate the list, in a temporary dropped after the call, and `listOf` would have to clone every element to return a list, or stay an intrinsic; nothing is saved, and keeping or returning the elements becomes impossible.
- **A slice or array type** (`&[T]`, `Array<T>`, stack-allocated): a new built-in type with its own borrow rules and layouts, for an allocation the zero-GC design already accepts per list; possible later as an optimization of reading-only callees (Tier 2 can also elide the allocation).
- **Borrowed element types** (`vararg names: &String` as a list of borrows): structs and lists cannot hold borrows (no lifetime annotations, the zero-cost memory invariant).

### D3. One vararg, anywhere; later parameters by name, trailing lambda or default (autonomous)

A function has at most one `vararg` parameter (`E2001` for a second, as Kotlin rejects it). It may sit anywhere in the list, as in Kotlin: positional arguments fill the parameters before it, then every further positional argument is one of its elements; the parameters after it take named arguments, the trailing lambda (the last one) or their defaults. This is Compose's `remember(vararg keys: Any?, calculation: () -> T)` shape. A vararg's elements are written one after another: a named argument between two of them is `E4004`, so the list is one argument evaluated at its first element's position, and evaluation stays in source order. A call with no element passes an empty list; a vararg is never "missing" and never has a default (`E2001` for one: the empty list is its default).

**Compile-time cost.** The binding pass of `check_call_arguments_with` stays one pass over the arguments.

Rejected:
- **Last parameter only, or last before a trailing lambda**: Kotlin allows any position, the binding rule is no more expensive, and a library signature such as `fun join(vararg parts: String, separator: String = "")` reads naturally.
- **Elements split around named arguments** (`f(1, last = 2, 3)`): the list's construction would interleave with another argument's evaluation; Kotlin rejects mixing there too.

### D4. Elements are typed as that many parameters of the element type, with the existing inference (autonomous)

A call with `n` elements is checked as a call of the declaration with the vararg replaced by `n` parameters of its element type, which is exactly how `listOf` is typed today (`infer_list_construction`), generalized to every declaration. Each element is a by-value argument of the element type, in a consuming position (a non-`Copy` place moves, a borrowed `Copy` value is copied out, `E4017` otherwise). For a generic element type the existing local inference applies unchanged: explicit type arguments win, the expected type fixes the result's type parameters, and the elements give lower bounds that are joined, left to right with the other arguments (`listOf(1, null)` is `List<Int?>`, `listOf(a, b)` of two `Todo`s is `List<Todo>`); a conflicting element is `E4004` at that element (`listOf(1, 2.0)`: there is no numeric widening), and an unsolved type parameter is `E4013` (`val xs = listOf()`), suggesting a type argument or annotation. A borrowed element gives its referent as the solution, as for any by-value type parameter (`generics`, borrow-free type arguments).

**Compile-time cost.** Linear in the arguments: one expanded signature of `n` element types (as today's `listOf`), the join of `n` lower bounds, no new inference, no cross-call analysis.

Rejected:
- **The first element fixes `T`** (no join): `listOf(1, null)` would need `listOf<Int?>`, a regression from today.
- **Numeric widening or a common supertype search**: Prism has neither, and either would be a global search per call.

### D5. Spread `*xs` passes a whole list, moved in, as the vararg's only argument (autonomous)

`*expr` at the start of an argument, Kotlin's spread, passes a `List<E>` as the whole vararg: `sum(*numbers)`, `listOf(elements = *xs)`. The list is moved in (a place moves, a temporary is consumed), never copied, so forwarding a vararg to another (`fun <T> bag(vararg xs: T): Bag<T> = Bag(listOf(*xs))`) costs nothing. Its type must be exactly the vararg's `List<E>` (lists are invariant; for a generic element it solves `T` exactly, as an argument of type `List<T>` does). A spread of a borrowed list is `E4017` with `clone()` help, as any move out of a borrow.

Restrictions, each `E4004`: a spread must be the vararg's only argument (`f(1, *xs)` and `f(*a, *b)` would need a concatenation, deferred); a spread for a parameter that is not a vararg (`g(*r)`, whose message says that reads through references are implicit); and, as in Kotlin, a named vararg argument that is not a spread (`listOf(elements = 1)`, help: `elements = *listOf(1)`).

`*` could not start an argument before this change (`E2001`: `*` writes through references only on the left of `=`), so the spread syntax is unambiguous; `val x = *r` and `return *r` stay `E2001`.

**Compile-time cost.** One token test per argument; one type comparison per spread.

Rejected:
- **No spread**: a vararg function could not forward its list, nor be called with a list built elsewhere.
- **A named argument taking the list without `*`** (`sum(xs = numbers)`): unambiguous too, but not Kotlin's form, which requires the spread and forbade single named elements.
- **Mixing elements and spreads by concatenation**: Kotlin allows it; it needs a runtime append that moves elements out of a consumed list, for a rare form. Deferred.

### D6. A trailing lambda never binds to a vararg (autonomous; changes `collections` D22)

A trailing lambda binds to the last parameter ("Trailing Lambdas Bind to the Last Parameter"); when that parameter is the vararg, the call is `E4004`, with help to pass the lambda inside the parentheses, as Kotlin rejects it ("passing value as a vararg is only allowed inside a parenthesized argument list"). So `listOf({ 1 }) { 2 }`, which `collections` D22 accepted as a two-element list, is rejected, and its scenario form changes. A vararg followed by a function-typed parameter takes the trailing lambda there (`fun on(vararg keys: Int, action: () -> Unit)` called as `on(1, 2) { }`).

**Compile-time cost.** One comparison per trailing lambda.

Rejected:
- **Keeping D22's rule** (the lambda as the last element): reads as a block after the call, which is why Kotlin forbids it, and would differ from Kotlin for every library vararg function.

### D7. Vararg is a property of the declaration, not of function types (autonomous)

Like parameter names and defaults, `vararg` belongs to the declaration (`SymbolKind::Function` gains `vararg: Option<usize>`, the parameter's index). The function's type has a `List<T>` parameter: `sum` as a value is `(List<Int>) -> Int`, and a call through a function value never packs (`f(1, 2)` is `E4004`; `f(listOf(1, 2))` works), as Kotlin's function references take the array. `vararg` in a function type (`(vararg Int) -> Unit`) is `E2001`. Generic vararg functions are named as values only with their call, as every generic function (`E4013`).

**Compile-time cost.** One optional index per function symbol.

Rejected:
- **Vararg function types**: a function value's calls would need the flag in every function type, its ABI and its conversions, for no library need.

### D8. Elements are stored list values (autonomous, follows from D2)

The elements are stored in a list, so they follow "Elements Are Stored Values" (`collections`): a closure element owns its captures, and one that may hold a borrow is `E5007`. The typed tree keeps today's shape for this: the list construction's declared parameters are the built-in's element type parameter, so the capture analysis and the borrow checker treat elements exactly as `listOf`'s arguments today, whether the vararg's element type is generic or not (`vararg actions: () -> Unit`).

**Compile-time cost.** None beyond today's.

### D9. Where `vararg` may appear (autonomous)

On parameters of `fun` declarations: top-level and extension functions, `compose fun` (the parameter is an ordinary `List<T>` parameter for the compose transform and its kept-argument rules), and `external fun` (passed as a list, by address). An `intrinsic` declaration carries it only when the catalog does, compared as the rest of its signature (`E4029`); no intrinsic has one. `E2001` for: a second vararg, a default value, `escaping` or `attached` in a vararg's type (the parameter is a list, not a function value the callee keeps; its function-typed elements are stored values, D8), a struct's constructor field, a closure parameter, a context parameter, and a function type's parameter.

**Compile-time cost.** Syntactic checks in the parser.

Rejected:
- **Varargs in struct constructors** (Kotlin allows them in class constructors): a struct field is a `List<T>` anyway, and construction would need the packing rules for constructors too; a follow-up if wanted.

### D10. `listOf` becomes `public fun <T> listOf(vararg elements: T): List<T> = elements` (autonomous)

`library/prism/collections.prism` declares `listOf` as an ordinary generic function; its intrinsic declaration, catalog entry and typing rule (`infer_list_construction` and the `ListOf` branch of `infer_call_expr`) are removed. `BuiltinFunction::ListOf` stays as the one construction the compiler lowers, renamed `BuiltinFunction::VarargList` (printed `builtin varargList`), reached only from a vararg's arguments, with the same lowering (D13). What `listOf` does is unchanged: arguments evaluated left to right, moved in, capacity exactly `n`, no allocation for `listOf()`; it now also takes a spread (`listOf(elements = *xs)` gives `xs` back), and its trailing-lambda form is rejected (D6).

Cost of the conversion: each element type a program uses instantiates `listOf` once (`listOf<Int>`, `listOf<Todo>`), a one-line instance planned from file summaries and cached as any instance; each call adds one call instruction moving the list's three words. Tier 1 accepts it; Tier 2 inlines it.

**Compile-time cost.** One small instance per element type per program; checking a `listOf` call costs what it costs today.

Rejected:
- **Keeping `listOf` an intrinsic beside `vararg`**: the agreed goal is that `listOf` is ordinary code.
- **Treating `listOf` specially in the backend** (inlining a known library function): a special case by name, which `vararg` exists to remove.

### D11. `print`, `println`, `remember` and `key` keep their rules (autonomous)

Kotlin's `println(message: Any?)` takes one value of any type, `remember(vararg keys: Any?, calculation)` and `key(vararg keys: Any?, block)` any number of values of any types. Prism has no `Any`, and a `vararg` is homogeneous (one element type), so none of them becomes a `vararg` function: `println(1)` must keep printing an `Int` without boxing it into a `List<Any>`. They stay rule-typed intrinsics; `library-in-prism` decides their final form (for example `println(message: &String = "")` with templates, as the rule "Output Built-In Arguments" already reads its argument's text form).

**Compile-time cost.** None.

Rejected:
- **`print(vararg values: T)`**: one type for all arguments, unlike Kotlin's one-argument `print`, and still not able to print an `Int` and a `String` in one call.
- **Variadic generics or an `Any` type**: a type-system feature far beyond this change.

### D12. No new error code (autonomous)

Every mistake fits an existing code: a misplaced `vararg`, a second one, a default or `escaping` on it, `vararg` in a function type or struct, are syntax errors (`E2001`); a trailing lambda for a vararg, a named non-spread vararg argument, a named argument between elements, a spread mixed with elements, a spread for a non-vararg parameter and an element of the wrong type are call shape errors (`E4004`); a borrowed element type is `E4022`; an unsolved element type `E4013`. The diagnostics catalog rows of `E2001`, `E4004` and `E4022` are widened.

**Compile-time cost.** None.

### D13. Typed tree, MIR and code generation: the vararg's argument is a built-in list construction (autonomous)

The typed tree normalizes a call as today, one argument per parameter (`typed-tree`, "Calls Are Normalized"): the vararg's argument is a `Call` of `Callee::Builtin(BuiltinFunction::VarargList)` with the elements in order and the element type as its type argument, of type `List<E>`, or the spread's operand consumed. `prism dump-typed-tree` prints `call main::sum` with its argument `call builtin varargList<Int>`. MIR lowers it like any argument: a `Call` terminator of `builtin varargList<E>` into a temporary, moved into the call (`_2 = call builtin varargList<Int>(const 1, const 2)`, then `call prism/collections::listOf<Int>(move _2)`); drop elaboration, the borrow checker and the compose transform see an ordinary built-in call and an ordinary move. Code generation keeps today's `ListOf` lowering under the new name. The MIR codec keeps the variant's tag (8), so only the name changes.

**Compile-time cost.** One node per vararg call, as `listOf` produces today.

Rejected:
- **Packing in MIR** (the call carrying its elements): every MIR consumer would learn a call with a variable number of operands for one parameter.
- **Desugaring in the parser to `listOf(...)`**: the parser does not know which callee has a vararg, and a library function would become a language dependency.

### D14. The ABI records vararg parameters (autonomous)

`AbiParameter` gains `is_vararg`, and the parameter's recorded type is its `List<E>`, so an importer builds the function's type unchanged and knows to pack. The ABI hash covers the flag: changing `xs: List<Int>` to `vararg xs: Int` changes what callers may write, so importers are re-checked; the machine signature is the same, so slot contracts (hot reload) are unaffected. `prism abi` prints `vararg xs: Int`.

**Compile-time cost.** One flag per parameter in Phase 1.

### D15. Formatting prints the new syntax only (autonomous)

`prism fmt` prints `vararg` and one space before the parameter's name, and `*` directly before a spread argument's expression (after `name = ` when named). The verifier compares the new AST fields. No layout rule changes.

**Compile-time cost.** None for compilation.

### D16. Compile timing (autonomous)

`crates/prism-cli/tests/compile_timing.rs` gains the shape `vararg-calls`: files of 256, 1,024 and 4,096 functions, each calling a generic and a non-generic vararg function with eight elements, a spread and a named argument after the vararg, asserting growth of at most ×4.6 per ×4 under `PRISM_ASSERT_TIMING=1`, as every stage's shapes do.

## Risks / Trade-offs

- **[An allocation per call]** A vararg function that only reads its elements still gets an owned list, allocated by the caller. → As in Kotlin; zero elements allocate nothing; a borrowed-slice form or Tier 2 elision is a follow-up if profiles show it.
- **[Elements are moved]** `log(name)` for `fun log(vararg parts: String)` moves `name`, where a Kotlin reader expects nothing to happen. → The ownership rule of every by-value argument; `E5001` on a later use names the move, and `name.clone()` or a `&String` parameter are the fixes. Documented in the rationale.
- **[`listOf` is now a call]** One more call per list built and one instance per element type. → Measured by `vararg-calls` and the existing list targets (the million-adds run builds one list); Tier 2 inlines.
- **[The trailing-lambda change]** `listOf({ 1 }) { 2 }` stops compiling. → Kotlin's rule; only its scenario fixture uses it; the message says to move the lambda inside the parentheses.
- **[Packets landing in between]** `owned-state` and `explicit-library` edit blocks this packet modifies. → The first task re-bases them.

## Migration Plan

No program changes meaning except the trailing-lambda form of `listOf` (D6), which becomes `E4004`. Fixture snapshots that print `builtin listOf` (typed-tree, MIR, CLIF) are re-blessed and checked to differ only by that name and the added call of `listOf`. Rollback: revert the commits.

## Open Questions

- Should a reading-only vararg avoid its allocation (a borrowed slice type, or Tier 2 eliding the list)? Revisit with profiles of `library-in-prism`'s code.
- When `library-in-prism` settles `print` and `println`, does Prism want a heterogeneous form (an `Any`-like type, or a template-based `println`)?
