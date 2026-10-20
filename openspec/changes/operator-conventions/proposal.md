# Proposal: operator-conventions

## Why

The post–Milestone 2 review agreed that the standard and compose libraries become "as much as possible ... normal prism code that is imported like any other code" (item **A4** of `docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20), and named the language additions that needs. The first is **L1, Kotlin-style `operator fun` conventions**: today operators, indexing, `for`, `in` and calls work only on built-in types, so a library type written in Prism cannot offer `16.dp + 8.dp`, `a < b` on two `Dp`s, `constraints[i]`, `for (m in measurables)` or `x in set`, and every such type would need a compiler intrinsic. This is item 7 of "After Milestone 2: the review round" in `ROADMAP.md` ("`get`, `set`, `iterator`, `plus`, `compareTo`, `equals`, `contains`, `invoke`"). The items after it (`drop-and-clone-hooks`, `vararg`, `library-in-prism`, `layout-in-prism`) rewrite library types in Prism and rely on these conventions.

## What Changes

- **`operator fun`.** A new contextual modifier, `operator`, marks an extension that operator syntax may call: `operator fun Dp.plus(other: Dp): Dp`. A function named `plus` without it is an ordinary function, as in Kotlin. Operator functions are extensions like any other: they resolve through the file's own declarations and its imports, by name, with one table lookup per use, and stay callable by name (`a.plus(b)`).
- **The conventions in scope**, with Kotlin's names and meanings:
  - arithmetic `a + b`, `a - b`, `a * b`, `a / b`, `a % b` → `plus`, `minus`, `times`, `div`, `rem`; unary `-a`, `!a` → `unaryMinus`, `not`;
  - ordering `a < b`, `a <= b`, `a > b`, `a >= b` → `a.compareTo(b)` compared with `0`;
  - equality `a == b`, `a != b` → a type's own `equals`, which replaces structural equality for that type everywhere `==` reaches it (fields of other types, nullables, boxes, lists, `when` values, compose's kept arguments and equal-write checks);
  - membership `a in b`, `a !in b` → `b.contains(a)`; **new syntax**: `in` and `!in` as binary operators (between `?:` and comparisons, non-associative); `x in xs` on a `List` uses its `contains` member, now declared `intrinsic operator`, and `x in a..b` on a `Range` is built in;
  - indexing `a[i, j]` → `get` (**new syntax**: several indices), returning a borrow or a value; a `get` returning `&E` makes `a[i]` a place behind that borrow, so `&a[i]`, `a[i].name` and method calls work without copying; a second `get` with a `mut` receiver returning `&mut E` serves writes (`a[i].done = true`, `&mut a[i]`, `mut fun` calls), as Rust's `Index`/`IndexMut`; `a[i] = v` and `a[i] += v` call `set` when declared;
  - calls `a(args)` on a value of a non-function type → `invoke`;
  - `for (x in y)` over a non-built-in type → a **cursor protocol** on the collection: `iterator()` returns a cursor value, `hasNext(cursor: &K)` and `next(cursor: &mut K)`, which returns the element (`&E` binds a borrow, as a list loop does), with a `mut next` for `for (x in &mut y)`. No allocation, no struct holding a borrow, and `break`/`continue` as in any loop.
  - compound assignment `a op= b` on such types is `a = a op b`, with `a` written once.
- **Built-in operators keep their meaning.** A built-in operator applies first whenever its operand types allow it, as a member wins over an extension; an operator extension it would always shadow (`operator fun Int.plus(other: Int)`, `get` on `List`) is `E4010`. Operator extensions on built-in types for other operand types work (`operator fun Int.times(other: Dp): Dp`, as in Compose).
- **Signatures are checked** with a new code, **`E4035` `InvalidOperatorDeclaration`**: each convention's arity, receiver modes, result (`compareTo` returns `Int`, `contains`/`hasNext` return `Bool`, the mutable `get` returns `&mut E`), and `equals` declared as `fun T.equals(other: &T): Bool` in `T`'s own file with `T`'s visibility, on a non-generic struct or enum.
- **Lowering.** Operator uses become ordinary calls in the typed tree, marked with the operator they spell, with receivers adjusted as for extension calls; MIR and the borrow checker see calls, plus one new loop shape for the cursor protocol. `==` stays an equality node; the code generator's equality expansion and glue call a type's `equals` where they meet that type. The ABI records the `operator` flag of exported functions and, in a type's definition, whether it has an `equals`.
- **Formatting** prints `operator` among the modifiers, index lists and `in`/`!in`; nothing else changes.
- **Not in scope (deferred, each a follow-up):** `rangeTo`/`rangeUntil` for custom ranges; `plusAssign` and the other in-place assignment operators; `unaryPlus`, `inc`/`dec` (`++` stays rejected); `componentN` (destructuring is by field name); `getValue`/`setValue` (delegated properties, C3); `in` and `!in` patterns over containers in `when`; generic `equals`; `compose` operators and operators with a context parameter; overloading one operator name by parameter type (one `Dp.times`); `hashCode`; Kotlin's iterator objects.

## Capabilities

### New Capabilities
- `operators`: the `operator` modifier and the conventions; resolution after built-in operators; operator uses as calls (receivers, arguments, evaluation order); signature rules (`E4035`); equality operators belonging to their type; ordering, membership, indexing (shared and mutable accessors, `set`), `invoke`, the cursor protocol of `for`, and compound assignment through operators; their compile-time cost.

### Modified Capabilities
- `syntax`: the `operator` modifier; `in` and `!in` as binary operators and the `!in` token; index lists `a[i, j]`.
- `type-system`: ordering comparisons, indexing and compound assignment accept operator functions.
- `collections`: indexing and `for` over types other than `List`; `in` on lists.
- `loops`: `for` over a type with the cursor protocol.
- `execution`: equality through a type's `equals`; running programs that use every convention.
- `typed-tree`: operator uses as marked calls; index places through accessors; cursor loops.
- `mir`: cursor loops lowered to calls of the protocol.
- `code-generation`: equality expansion and glue calling a type's `equals`.
- `compilation-and-abi`: operator flags and equality operators in the ABI.
- `standard-library`: intrinsic operators (`List<T>.contains`).
- `formatting`: operator modifiers, index lists and membership operators printed.

## Impact

- `crates/prism-lexer` (`!in` token), `crates/prism-ast` (`FunctionDecl::is_operator`, `BinaryOp::{In, NotIn}`, `ExprKind::Index` with an index list), `crates/prism-parser/src/parser.rs` (modifier, precedence, index lists).
- `crates/prism-abi/src/signature.rs` (`ExportedSymbol::Function::is_operator`, an `equality` flag on struct and enum symbols and on `AbiTypeDefinition`).
- `crates/prism-typechecker/src/checker.rs` (`infer_binary_op`, `infer_unary_op`, `infer_index`, `infer_place`, `check_assignment`, `check_compound_assignment`, `check_iterable`, `infer_call_expr`, `register_extension_decl`, `find_extension`), a new `crates/prism-typechecker/src/operators.rs` (conventions, signature checks), `error.rs` (`E4035`).
- `crates/prism-typed-tree` (call marks, `ForIteration::Cursor`, dump), `crates/prism-mir/src/build` (cursor loops, compound assignment through operator functions), `crates/prism-codegen/src/equality.rs` and `glue.rs` (calls of `equals`).
- `crates/prism-formatter` (modifier, index lists, `in`/`!in`), `library/prism/collections.prism` (`contains` as `intrinsic operator`) and the intrinsic catalog.
- `crates/prism-cli/tests/fixtures/` (new cases for every scenario, executable ones under `--check-allocations`), `crates/prism-cli/tests/compile_timing.rs` (the `operator-uses` shape).
- `openspec/specs/diagnostics/spec.md` (`E4035`, and the `E4002`, `E4003`, `E4004` and `E4010` rows), `AGENTS.md` (the new `operators` spec in the reference list).
