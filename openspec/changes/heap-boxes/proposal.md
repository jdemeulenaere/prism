# Proposal: heap-boxes

## Why

`Heap<T>` is how Prism gives a recursive type a finite layout (`enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }`), and the zero-cost memory safety invariant names it as the single-owner heap allocation of the language. Today the type exists but no value of it can: `Heap(1)` is `E4001`, a box cannot be passed where its contents are borrowed (`eval(h)` for `fun eval(e: &Expr)` is `E4004`), `h + 1` is `E4002`, `*h = v` is `E4019`, nothing can move the contents out, a `consuming fun` cannot be called through a box (`E4017`), and the backend rejects every `Heap<T>` value with `E6001`. So expression trees, linked lists and trees cannot be written. This change is item 17 of "Then: the road to Milestone 2" in `ROADMAP.md`, and the deferred item "`Heap<T>` construction and unboxing" below it. It follows the design that `runtime-strings-and-drops` recommended (its D19) and settles the questions that change, `reference-access`, `extension-methods` and `payload-patterns` left for it.

## What Changes

- **Construction.** `Heap(value)` allocates a box and moves the value into it, and `Heap<Int?>(1)` names the contents' type. The expected type also gives it (`val h: Heap<Int?> = Heap(1)` converts `1` before boxing). `Heap` is a built-in constructor: a top-level declaration named `Heap` is `E4010`, and `Heap` named without a call is `E4001`.
- **Reading through a box, as through a reference.** Operators and conditions read through `Heap<T>` layers (`h + 1`, `if (flag)`), a `Copy` content is copied out where a `T` is expected (`val n: Int = h`), `print`, `toString()` and `length` read through boxes, and fields and receivers already do. A `Heap<T>` passed where a `&T` is expected lends its contents (`eval(h)`), and `&Heap<T>` converts to `&T`, `&mut Heap<T>` to `&mut T` or `&T`, and `Heap<T>?` to `(&T)?`, wherever such a type is expected.
- **Writing the contents.** `*h = v` writes a box's contents, dropping the old contents and keeping the allocation, when `h` is a mutable place (a `var`, a `var` field path, or reached through `&mut`). `*h += 1` works too. `*` peels one layer, so `**r = v` writes through a `&mut Heap<T>` and `*r = Heap(v)` replaces the box. `E4019` now says that `*` needs a reference or a `Heap`. `h = 5` for a `var h: Heap<Int>` is `E4002`, with help suggesting `*h = 5`.
- **Moving the contents out.** The built-in consuming member `h.unbox(): T` moves the contents out and frees the box, and `n?.unbox()` gives a `T?`. A `consuming fun` called through boxes unboxes its receiver (`h.toAccount()` for `h: Heap<User>`), as Rust does. **BREAKING (rejection lifted):** this was `E4017`. Through a reference both stay `E4017`. Taking a non-`Copy` content where a `T` is expected (`val u: User = h`) is `E4017`, naming "the contents of `Heap` `h`", with help suggesting `h.unbox()`.
- **BREAKING: boxes are invariant.** `Heap<U>` is assignable to `Heap<T>` only for the same type, as references and generic types are, because `Heap<Int>` and `Heap<Int?>` hold contents of different layouts. Once boxes are invariant, the only `Subtype` coercion left in the typed tree that changes a representation is function-type variance.
- **Patterns see through boxes** (on top of `payload-patterns`). A constructor, value or range pattern at a `Heap<T>` part, or of a `Heap<T>` subject, matches the contents (`Expr.Add(Expr.Lit(0), val r)`), while `_` and bindings take the box itself, with the subject's binding mode. Taking a value apart through a box unboxes that box. Exhaustiveness treats a box as its contents. Recursive evaluation reads naturally: `when (e) { Expr.Lit(val n) -> n; Expr.Add(val l, val r) -> eval(l) + eval(r) }` for `e: &Expr` binds `l: &Heap<Expr>` and passes it on as a `&Expr`.
- **Typed tree, MIR, borrow checker.** Two new built-ins, `Heap` (construction) and `unbox`, are calls. A box's contents are the `Deref` place of the box, which already exists, and moves out of it stay invalid in MIR, so only `unbox` moves contents. The borrow checker already treats the contents as owned by the box (`&mut h.x` conflicts with `h`, and `h` cannot be reassigned or moved while its contents are borrowed). Those rules are now pinned by scenarios. Boxing a capturing closure is `E5007`, like storing it in a struct.
- **Backend.** A box is a non-null pointer to an allocation of exactly the contents' size and alignment, made through the runtime allocator. The argument is evaluated before allocating. A zero-sized content allocates nothing, and running out of memory is the runtime error "out of memory". `Heap<T>?` stays one pointer. Dropping a box runs its contents' drop glue and then frees it, and cloning allocates a new box holding a clone. Comparing two boxes of structs or enums calls a new per-type equality glue function, `<module>#equals(<type>)`. Equality, drop and clone glue check the stack, and a stack overflow inside glue is reported without a location. `E6001` no longer names `'Heap' values`.
- **Fixtures.** Every runtime scenario is an executable fixture run under `--check-allocations`. That covers expression trees evaluated recursively, linked lists, trees, writes, unboxing, cloning, equality and zero-sized contents, so leaks and double frees are caught.
- Not in scope: iterative drop of long chains (a follow-up; deep structures are a "stack overflow" for now), niches for other types, `Heap` receivers for extensions (`fun Heap<T>.f()`, `E2001`), a read form of `*`, implicit unboxing at by-value positions, placement construction (building the value directly in the box), and shared or reference-counted boxes, which the zero-cost memory safety invariant forbids.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `type-system`: `Heap(value)` construction, the reserved name `Heap`, operators reading through boxes, invariant boxes, `toString()` and the output built-ins through boxes.
- `references`: copy-out through boxes, a box lending its contents at reference parameters and by conversion, writing a box's contents with `*`, borrows of a box's contents, and boxing a capturing closure as an escape.
- `ownership`: `unbox()`, consuming receivers through boxes, and moving a box's contents as `E4017`.
- `extension-functions`: the built-in `unbox()` takes precedence over extensions.
- `patterns`: patterns look through boxes, taking apart through a box unboxes it, and exhaustiveness through boxes.
- `typed-tree`: the `Heap` and `unbox` built-ins, and boxes in the typed tree.
- `mir`: calls of the new built-ins, a box's contents as a place, and unboxing when a pattern takes a value apart through a box.
- `code-generation`: the representation of a box, its glue, equality glue, glue that checks the stack, and the `E6001` list without `Heap<T>`.
- `execution`: the runtime meaning of construction, reading, writing, unboxing, dropping, cloning and comparing boxes, and a stack overflow in glue.

## Impact

- `crates/prism-typed-tree`: `BuiltinFunction::HeapAllocate` and `Unbox`, with dump names and validation.
- `crates/prism-typechecker`: construction in `infer_call_expr`, `unbox` in `infer_method_call`, unboxing consuming receivers in `pass_receiver`, copy-out, contents borrows and the box view in `coerce`, `copy_out` and `implicit_borrow`, operators through boxes, `*` through boxes in `assignment_target`, the `Heap` covariance rule removed from `types.rs`, the reserved name, and patterns and exhaustiveness through boxes in `patterns.rs` and `exhaustiveness.rs` (from `payload-patterns`).
- `crates/prism-mir`: lowering and validation of the new built-in calls, and unboxing in the pattern lowering (`build/pattern.rs`).
- `crates/prism-borrowck`: loans and escapes through the new built-ins.
- `crates/prism-codegen`: allocation and unboxing in `lower.rs`, box glue and equality glue in `glue.rs`, the stack check in glue, `Heap<T>` accepted by `supported.rs`.
- `crates/prism-runtime`: a runtime entry raising "stack overflow" without a location.
- `crates/prism-cli`: fixtures under `type-system/`, `references/`, `ownership/`, `extension-functions/`, `patterns/`, `typed-tree/`, `mir/`, `code-generation/` and `execution/`; the `E6001` fixtures that pinned `Heap<T>` move to function values; `examples/run/` evaluates a boxed expression tree.
- Specs: the deltas above. The diagnostics catalog rows for `E4010`, `E4017`, `E4019` and `E6001` are widened or narrowed, edited directly.
- `docs/architecture.md`: on archive, §3's "Recursive Heap Indirection via Heap<T>" and the `Heap<T>` sentences that `runtime-strings-and-drops` moved there go into the specs.
- **Prerequisites:** `codegen-aggregates`, `runtime-strings-and-drops` (allocator, drop and clone glue, `--check-allocations`) and `payload-patterns` (patterns that bind payloads) must be archived first. Several deltas modify their requirements. `payload-patterns` is item 21 in `ROADMAP.md` today, so it has to land before this change, item 17, which means reordering the roadmap (D1).
