# Proposal: vararg

## Why

The post–Milestone 2 review agreed that the standard and compose libraries become "as much as possible ... normal prism code that is imported like any other code" (item **A4** of `docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20), and named the language additions that needs. The fourth is **L4, `vararg`**: today `listOf(1, 2, 3)` is a rule-typed intrinsic of `/prism/collections` (`standard-library`, "Intrinsic Calls Are Typed by Their Signatures"; `collections` D4, "the one variadic built-in ... becomes an ordinary library function once `vararg` exists"), so no Prism function can take a variable number of arguments, and the library cannot write `listOf` or a helper like it. This is item 9 of "After Milestone 2: the review round" in `ROADMAP.md` ("variadic parameters, so `listOf` is ordinary code"); `library-in-prism` (item 10) comes after it.

## What Changes

- **`vararg` parameters**, Kotlin's syntax: `fun <T> listOf(vararg elements: T): List<T>`. `vararg` is a contextual modifier before a parameter's name, on parameters of `fun` declarations only (top-level, extension, `compose`, `external`), at most one per function, without a default value, `escaping` or `attached` (`E2001` otherwise).
- **Inside the body the parameter is an owned `List<T>`**, built by the caller with exactly the written elements (one allocation, none for zero elements) and moved into the callee, which may read, change, keep or return it: `listOf`'s body is `= elements`. The element type must be borrow-free (`E4022`), as every list's.
- **Calls**, Kotlin's rule: positional arguments from the vararg's position on are its elements, written one after another; parameters after the vararg are given by name, by the trailing lambda or by their default. No elements is an empty list. A trailing lambda never binds to a vararg (`listOf({ 1 }) { 2 }` becomes `E4004`, as in Kotlin).
- **Elements are typed as that many parameters of the element type**, exactly as `listOf` is typed today: each element is a by-value argument; for a generic element type, the written type argument, the expected type and the elements' joined types give `T` with the existing local inference (`listOf(1, null)` is `List<Int?>`, `listOf(1, 2.0)` is `E4004` at `2.0`, `val xs = listOf()` is `E4013`). No new inference.
- **Spread** `*xs`, Kotlin's syntax, passes a whole `List<T>` as the vararg, moved in without copying: `sum(*numbers)`, `listOf(elements = *xs)`. It is the only argument of the vararg (mixing `f(1, *xs)` is deferred), and a named vararg argument must be a spread, as in Kotlin. `*` starting an argument was `E2001` (`*` writes through references only on the left of `=`), so the syntax is free.
- **Vararg belongs to the declaration**, like parameter names and defaults: a function value of `fun sum(vararg xs: Int): Int` has type `(List<Int>) -> Int`, and calls through function values never pack.
- **`listOf` becomes ordinary Prism**: `public fun <T> listOf(vararg elements: T): List<T> = elements` in `library/prism/collections.prism`. The intrinsic, its catalog entry and its typing rule are removed; the compiler keeps one built-in, the construction of a vararg's list (`builtin varargList` in dumps), since `List` stays Rust-backed (no `unsafe` for now, user decision 2026-10-20).
- **`print`, `println`, `remember` and `key` stay rule-typed**: they take arguments of different types (Kotlin types them with `Any?`), which a homogeneous `vararg` cannot express; `library-in-prism` decides their form.
- **ABI**: each exported parameter records whether it is `vararg`, covered by the ABI hash; `prism abi` prints `vararg xs: Int`.
- **Lowering**: the typed tree gives the vararg one argument, the built-in list construction of its elements (or the spread list); MIR builds it into a temporary moved into the call; code generation reuses today's `listOf` lowering unchanged.
- **Formatting** prints `vararg` before the parameter name and `*` before a spread argument.
- **No new error code**: the mistakes are syntax (`E2001`), call shape (`E4004`), borrowed element types (`E4022`) and unsolved element types (`E4013`).
- **Not in scope (deferred, each a follow-up):** spreads mixed with elements or several spreads (`f(1, *xs, 2)`), spreading a borrowed list without moving it, `vararg` in struct constructors and function types, `print`/`println`/`remember`/`key` as `vararg` functions, `mutableListOf`/`setOf`/`arrayOf`, and borrowed (non-owning) varargs.

## Capabilities

### New Capabilities
- `vararg-parameters`: `vararg` parameters: the parameter as an owned `List<T>`; binding arguments to it (elements, parameters after it, contiguity, empty calls); typing elements as parameters of the element type with the existing inference; spread arguments; trailing lambdas never binding to it; vararg as a property of declarations, not function types; elements as stored list values; the compile-time cost.

### Modified Capabilities
- `syntax`: the `vararg` modifier and where it may appear; spread arguments; `vararg` as a contextual keyword; `*` at the start of an argument no longer `E2001`.
- `type-system`: trailing lambdas bind to the last parameter unless it is a vararg.
- `collections`: `listOf` as an ordinary generic function with a `vararg` (named spreads accepted).
- `standard-library`: `listOf` no longer rule-typed; `/prism/collections` declares it in Prism.
- `typed-tree`: a vararg's argument is a built-in list construction or the spread list.
- `mir`: the vararg list built into a temporary; list operations without `listOf`.
- `code-generation`: the capacity rule stated for a vararg's list.
- `compilation-and-abi`: vararg flags in the ABI.
- `execution`: vararg calls run, moving and freeing their elements once.
- `formatting`: `vararg` and spread arguments printed.

## Impact

- `crates/prism-ast` (`Param::is_vararg`, `CallArg::is_spread`), `crates/prism-parser/src/parser.rs` (`parse_params`, `parse_call_args`, the `*`-at-expression-start error), `crates/prism-formatter` (parameters, arguments, `verify.rs`).
- `crates/prism-abi/src/signature.rs` (`AbiParameter::is_vararg`, hashing, printing; Phase 1 recording `List<T>` for the parameter).
- `crates/prism-typechecker/src/checker.rs` (`register_function_decl`, the ABI ingestion of functions, `SymbolKind::Function` gaining `vararg: Option<usize>` in `scope.rs`, `callee_declaration`, `check_call_arguments_with`, `check_generic_call_arguments`, `call_extension`; `infer_list_construction` and the `ListOf` branch of `infer_call_expr` removed), `intrinsics.rs` (catalog entry removed).
- `crates/prism-typed-tree/src/tree.rs`, `validate.rs`, `dump.rs` (`BuiltinFunction::ListOf` renamed `VarargList`, printed `varargList`), `crates/prism-mir` (`codec.rs`, `validate.rs`), `crates/prism-codegen/src/lower.rs` (the renamed variant only).
- `library/prism/collections.prism` (`listOf` in Prism), `crates/prism-cli/tests/library_tests.rs` if it pins the source.
- `crates/prism-cli/tests/fixtures/` (new `vararg-parameters/` cases, executable ones under `--check-allocations`; the `list-of` form of the trailing-lambda scenario becomes a rejection; MIR and typed-tree snapshots naming `builtin listOf` re-blessed), `crates/prism-cli/tests/compile_timing.rs` (`vararg-calls`).
- `openspec/specs/diagnostics/spec.md` (`E2001`, `E4004` and `E4022` rows widened), `AGENTS.md` (the new spec in the reference list).
