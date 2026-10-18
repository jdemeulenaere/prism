# Design: collections

## Context

See `proposal.md` for motivation. This is item 24 of "Then: the road to Milestone 2" in `ROADMAP.md` (added on 2026-10-18, process row P6), and the deferred item "Iterating anything other than `Range` in `for` loops". Earlier changes left room for it: `typed-tree` and `mir-lowering` reserved an index place for "collections whose elements are stored", `runtime-strings-and-drops` (D4, D5) kept `s[i]` a call so that `xs[i]` could be a place, and the ROADMAP follow-up on a character iterator waits for "the iteration protocol of `collections`". `docs/architecture.md` §5 plans `for (todo in todos.value)` over a `State<List<Todo>>` and a `standard-library` change bundling Prism source with `intrinsic` declarations, which raises the question of where `List<T>` should live (D1). The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

**Baseline.** This change was proposed while `payload-patterns` was being implemented and `heap-boxes`, `string-templates`, `monomorphization`, `closures` and `escaping-closures` were proposed but not implemented. `payload-patterns`, `heap-boxes`, `monomorphization`, `closures` and `escaping-closures` land before it and are needed: lists in generic code run as instances, list glue extends the equality glue and stack checks of `heap-boxes`, indexing reads through boxes as `heap-boxes` makes every implicit read do, and lists of function values need closures that own their captures. `string-templates` and `instance-cache` are not needed. Almost every delta is ADDED. The three MODIFIED requirements, `loops` "For Loops Over Ranges" and `typed-tree` "Loops and Jumps Are Resolved" and "Assignments Are Statements Over Places", are touched by no pending change. When implementing, re-read the diagnostics catalog and the `E6001` list as those changes leave them.

What exists today:

- **Type checker** (`crates/prism-typechecker`). `List` is not a known type: an annotation `List<Int>` lowers to an opaque name and `listOf` is `E4001`. `for` checks its iterable against the built-in `Range` (`check_statement`, `StatementKind::For`, through `check_operand`) and binds an `Int`. `ExprKind::Index` accepts only a `String` and builds a call of `BuiltinFunction::StringCharacter` (`infer_expr`). Built-in members are special cases: `length` in `build_member_access`, `clone` and `toString` at step 2 of `infer_method_call`. Methods are `Extension` records (mode, receiver type, module, symbol) keyed by `ReceiverKey` (a nominal id, or the whole type otherwise), looked up by `find_extension`, passed by `pass_receiver` and `apply_receiver_adjustment`, and called by `call_extension`. `Range` is a built-in nominal (`NominalId::builtin_range()`), `Heap<T>` a structural `Type::Heap`.
- **Typed tree** (`crates/prism-typed-tree`). `ExpressionKind::Index { base, index }` exists, "reserved for collections", and the type checker never builds it. `StatementKind::For { variable, range, body }`. `BuiltinFunction` has `Clone`, `Print`, `PrintLine`, `StringLength`, `StringCharacter`, `ToString` (and `HeapAllocate`, `Unbox` after `heap-boxes`).
- **MIR** (`crates/prism-mir`). `Projection::Index(LocalId)` exists; `build/place.rs` already lowers an `Index` node by evaluating the index into a temporary, but the validator rejects any index projection (`projection_type` gives `None`). `lower_for` (`build/control_flow.rs`) lowers range loops with a latch that compares before incrementing.
- **Borrow checker** (`crates/prism-borrowck`). `places.rs` already treats two index projections as overlapping (`is_prefix`, the conflict test) and has a unit test for it.
- **Backend and runtime.** `lower.rs::location` returns an internal error for an index projection. Glue (`glue.rs`) has drop and clone kinds over `owns_storage`; `heap-boxes` adds equality glue and stack checks. The runtime has the tracked allocator (`allocation.rs`), runtime errors with locations (`panic.rs`) and strings (`string.rs`).

## Goals / Non-Goals

**Goals:**
- A growable list that holds any borrow-free type, nests, sits in structs, enums, boxes, top-level values and generic code, and is built, read, written, iterated, compared, cloned and dropped with every allocation freed exactly once, which `--check-allocations` checks in every fixture.
- Elements that are places, so lists of structs are updated in place and borrowed without copies, with no rule the borrow checker does not already have.
- `for (x in list)` that reads like Kotlin, statically free of iterator invalidation.
- `add` at native speed: a million adds well under the 20 ms target.
- A design that moves unchanged into the bundled library when `standard-library` lands.

**Non-Goals:**
- Higher-order helpers (`forEach`, `map`, `filter`, `any`, `joinToString`...), which belong in the bundled library (D17).
- A user-visible iteration protocol, consuming iteration, `String` iteration (D10).
- `List(size) { init }`, `vararg`, `remove(element)`, `first()`, `last()`, `getOrNull`, `subList`, slices, sorting, capacity control, maps and sets, printing lists.
- Optimizations: eliding bounds checks a loop proves, a null niche for `List<T>?` (D13), small-buffer lists, sharing glue between element types of the same layout.

## Decisions

### D1. A built-in type now, shaped for the bundled library (autonomous)

`List<T>` is a compiler built-in, as `String` and `Heap<T>` are, and its members are built-in members. The question was whether it should wait for `standard-library` (item 26) and be declared there in Prism, as `docs/architecture.md` §5 plans for `State<T>` ("intrinsic generic types of `/prism/compose`"). It should not wait:

- **It cannot be written in Prism.** A list needs raw, uninitialized, growable memory and element moves by address. Prism has no unsafe layer and will not have one before self-hosting. So its storage operations (`add`, element addresses, growth, glue) are lowered by the compiler wherever its declaration lives, and an `intrinsic struct List<T>` would only move the signatures into a source file.
- **It is needed first.** Items 25 to 33 use it: `state-handles` and the todo app hold `List<Todo>`, and the compose prelude needs collections of children and handlers. Waiting would reorder the roadmap to put a large library change before a small, self-contained one.
- **It is shaped to move.** The members are `Extension` records with receiver modes and generic signatures, the same data a declaration `intrinsic mut fun <T> List<T>.add(element: T)` in a bundled file would produce (D6). `standard-library` can then declare them in source, and the higher-order helpers that can be written in Prism go there as ordinary code (D17). The semantics stay; only where the declarations come from changes.

Rejected: waiting for `standard-library` and declaring `List<T>` there (above); a library type written in Prism over an unsafe layer (none exists); a fixed-capacity array type first (Kotlin users expect a growable `List`, and UI children and todo items have no fixed count).

### D2. `Type::List`, a structural built-in type with a reserved name (autonomous)

The type is a new variant `Type::List(Box<Type>)` in `prism-typed-tree`, like `Type::Heap`, with `AbiType::List` in `prism-abi`. Every exhaustive match over types then has to decide what a list is (layout, glue, markers, substitution, borrow-freedom, rendering, ABI), and the compiler finds each. A nominal `<builtin>.List` struct, as `Range` is, would instead enter every struct path (constructors, field access, struct patterns, destructuring, layout from fields) as a struct with no fields, and each would need a special case keyed on its id that nothing forces.

The name `List` is reserved like `Heap` (`heap-boxes`, D2): a top-level declaration or type parameter named `List` is `E4010`, and `List` as a value or callee is `E4001` with help naming `listOf`. Reserving the callee too keeps `List(size) { init }` addable later without breaking anything. `Range` is shadowable, but a shadowable `List` would make `List<Int>` mean different things in different files, and the future prelude can still shadow it explicitly if `standard-library` decides so.

Extensions on lists are keyed by a new `ReceiverKey::List` (the type constructor without its argument), so `fun <T> List<T>.second()` and `fun List<Int>.sum()` share a key as the extensions of a generic struct do (`generics`, D10). The parser's `Heap<...>` receiver rejection does not apply to `List`.

Lists are an indirection: `E4023` (`InfinitelySizedType`) does not follow a list's element type, so `struct Node { val children: List<Node> }` is accepted. `monomorphization`'s rejection of non-regular generic types ("Non-Regular Generic Types Are Rejected") must look through `List` as it looks through `Heap`, since `struct Nest<T> { val inner: List<Nest<List<T>>> }` would need infinitely many instances of its glue.

Rejected: a nominal built-in struct (above); a shadowable name like `Range`; `Vec` or `ArrayList` (Kotlin users write `List`).

### D3. One list type; mutability comes from the place (autonomous)

There is one `List<T>`. A list is changed through a mutable place, `var xs`, a `var` field, or a `&mut List<T>`, exactly as a struct's fields are, and a function that only reads takes `&List<T>`. Kotlin splits `List` and `MutableList` because its lists are shared references, where the type is the only thing that stops an alias from mutating; with single ownership and exclusive `&mut`, a `&List<T>` already is the read-only view, statically. A second type would double the API, need conversions, and add nothing the borrow checker does not already guarantee.

The cost for Kotlin users is `var xs = listOf<Int>()` where Kotlin writes `val xs = mutableListOf<Int>()`; a mutating call on a `val` list is `E4003` with help saying that a list is changed through a `var` or a `&mut`. Rejected: Kotlin's `List`/`MutableList` pair; Swift's `let`/`var` arrays (which is in effect this design).

### D4. Construction with a variadic built-in `listOf` (autonomous)

`listOf(e1, ..., en)` is a built-in function typed, at each call, as a generic function of exactly `n` parameters of type `T` returning `List<T>`. That reuses generic call checking completely: explicit type arguments, inference from the expected type and from the joined arguments (`listOf(1, null)` is a `List<Int?>`), `E4004` for a conflicting argument and `E4013` for an unsolved `T`. The built-in is found last, like `print` (`codegen-core`, D15), so a user declaration named `listOf` shadows it. It is a call of `BuiltinFunction::ListOf` with type argument `[T]`; the backend allocates exactly `n` elements once and moves the arguments in, in order.

Prism has no `vararg`; `listOf` is the one variadic function, as `print` is the one with an optional argument. When `vararg` exists, `listOf` can become an ordinary bundled function. Rejected: `mutableListOf` (D3); `List<Int>()` or `ArrayList()` constructors (no constructor overloading, and `List` is reserved for `List(size) { init }`); `emptyList()` (`listOf()` covers it); adding `vararg` here (a language feature of its own, recorded as a follow-up).

### D5. The member set and its names (autonomous)

Kotlin's names, for its `List`/`MutableList` members, with Prism's ownership:

| Member | Mode | Signature | Note |
|---|---|---|---|
| `size` | shared | `Int` | read without parentheses, like `length` |
| `indices` | shared | `Range` | `0..<size`, for index loops that mutate |
| `isEmpty()`, `isNotEmpty()` | `fun` | `(): Bool` | |
| `contains(element)` | `fun` | `(element: &T): Bool` | needs `==` on `T` (D12) |
| `indexOf(element)` | `fun` | `(element: &T): Int` | -1 when absent, as Kotlin |
| `add(element)` | `mut fun` | `(element: T)` | returns `Unit`, not Kotlin's always-`true` `Boolean` |
| `insert(index, element)` | `mut fun` | `(index: Int, element: T)` | Kotlin's `add(index, element)` |
| `removeAt(index)` | `mut fun` | `(index: Int): T` | gives the element by move |
| `removeLast()` | `mut fun` | `(): T` | |
| `reverse()` | `mut fun` | `()` | in place |
| `clear()` | `mut fun` | `()` | keeps the capacity |
| `clone()` | `fun` | `(): List<T>` | the built-in `clone()`, when `T` is `Clone` |

`insert` replaces Kotlin's `add(index, element)` because Prism has no overloading (`E4010` for two extensions of one name), as Rust and Swift name it. `contains` and `indexOf` take `&T`, so `names.contains(name)` borrows `name` (implicit borrow at argument sites) instead of moving it, and `xs.contains(3)` borrows a temporary. `removeLast` and `reverse` are included as the building blocks that let the bundled library write an order-preserving, O(n) consuming `filter` in Prism (D17). There is no `get(i)`/`set(i, v)`: indexing is the way (D7).

Rejected: `add(index, element)` (overloading); `remove(element)` (ambiguous next to `removeAt` with an `Int` list, Kotlin's own pitfall; a follow-up); `first()`, `last()` and `getOrNull(i)` returning `&T` or `(&T)?` (useful, but cheap to add later, and `xs[0]` and `xs[xs.size - 1]` serve now).

### D6. Members are built-in extension records that win over extensions (autonomous)

The type checker keeps a table of the list members as `Extension` records whose receiver is `List<T>` (with `T` the extension's type parameter), whose module is the built-in module, and which carry a new `builtin: Option<BuiltinFunction>`. `infer_method_call` consults the table after fields, `clone()` and `toString()` and before user extensions, and then runs the ordinary extension path: `pass_receiver` borrows the receiver by its mode (a `mut fun` needs a mutable place, `E4003`), receivers look through `&`, `&mut` and `Heap<T>` layers, safe calls work, arguments are checked with generic inference against the receiver's `T`, and `call_extension` emits `Callee::Builtin` instead of `Callee::Extension`. `size` and `indices` are read in `build_member_access` as `length` is. An extension on `List` named like a member could never be called, so it is `E4010` (`UncallableExtension`), as for `length` on `String`.

This costs one table and one `Option`, and puts every list member through the code paths every extension uses, including "Mutable Receivers Are Borrowed After the Arguments", so `xs.add(xs.size)` is accepted. It is also exactly what `intrinsic` extension declarations in a bundled file would build (D1). Rejected: a special case per member, as `length` and `clone` are (a dozen special cases repeating receiver passing); resolving the members as imported extensions that a file's own extension could shadow (that would let a file redefine `add`; whether a prelude's members can be shadowed is for `standard-library` to decide).

### D7. Elements are places (autonomous)

`xs[i]` is `ExpressionKind::Index { base, index }`, the place that `typed-tree` reserved, over the list place read through any `Deref`s of `&`, `&mut` and `Heap<T>` layers. Everything places already do then applies to elements with no new rule: a `Copy` element is copied out (`Copy-Out From References`), a non-`Copy` element in a consuming position is `E4017` (its source "an element of `xs`", help `xs[i].clone()` for a `Clone` type and `xs.removeAt(i)`), `&xs[i]`, `&mut xs[i]` and implicit argument and receiver borrows borrow in place, `xs[i] = v` drops the old element ("Assignments Drop the Old Value"), compound assignments read and write it, and `xs[i].done = true`, `xs[i].add(1)` and `grid[i][j] = 0` work through field and receiver paths. Writing needs every layer above the element to be writable, by the rule field writes already follow (`contents_writable` after `heap-boxes`).

This is what `runtime-strings-and-drops` (D5) anticipated: a string's character is computed, a list's element is stored. Rejected: Kotlin's `get`/`set` operator calls (a read would be a value, so `xs[i].done = true` would write a temporary and lists of non-`Copy` structs could not be updated in place); a borrow-returning `get(i): &T` member (a place gives the same borrow and also writes).

### D8. Bounds checks at the access, as runtime errors (autonomous)

Every computation of an element's address checks `index < size` as an unsigned compare (a negative index is a huge unsigned value), so reads, writes, borrows and the loop's own element borrows are all checked. A failure calls `prism_rt_list_index_out_of_range(index, size, file, file_length, line, column)`, which reports "list index <i> out of range for size <n>" at the source of the access and exits with status 101, like "string index <i> out of range for length <n>". `insert` accepts `0..size` inclusive, and `removeLast()` of an empty list reports "removeLast() on an empty list" (Kotlin's `NoSuchElementException` "List is empty."). Checks run after the operands of the access are evaluated (so `xs[f()] = g()` runs `f` and `g` first, as Kotlin's `set(f(), g())` does) and before the list changes. Runtime errors do not unwind, so no partially changed list is ever observed.

Rejected: unchecked indexing (memory unsafety, against the zero-cost memory safety invariant); `getOrNull` as the only access (Kotlin users index); a location-free error (string indexing set the precedent of locating it).

### D9. `for (x in list)` borrows the list and binds `&T` (autonomous)

The iterable is evaluated once and borrowed for the whole loop: shared, unless its type is `&mut ...` (as `&mut xs`, or a `&mut List<T>` parameter, is), and then mutably. A place is borrowed where it lives; any other iterable is a temporary, which the `for` statement keeps until the loop ends, as every temporary lives to the end of its statement. The loop variable is a fresh immutable `&T` (or `&mut T`) per iteration, whatever `T` is.

Binding `&T` even for `Copy` elements keeps one rule: the variable's type does not depend on `T`, generic code over `List<T>` reads like concrete code, and nothing is copied. It costs nothing in practice, because operators, conditions, `print`, and positions that expect a `T` all read through references (`sum += n`, `if (n > 2)`, `ys.add(n)`), so `for (n in numbers) total += n` reads as in Kotlin. `for (x in &mut xs) { *x += 1 }` and `for (t in &mut todos) t.done = true` update in place, with `*` because writing through a reference is explicit. The loop's borrow is an ordinary borrow, so iterator invalidation (`xs.add` inside `for (x in xs)`) is `E5003` with no new rule (D11).

Rejected: copying `Copy` elements (`x: Int`), which makes the variable's type depend on `T`; consuming iteration as the default (Rust's `for x in v`, which Kotlin users would find moves their list), a follow-up as an explicit form; Kotlin's semantics of iterating a snapshot (needs shared ownership).

### D10. No iteration protocol yet: two built-in iterables (autonomous)

`for` knows two iterables, `Range` and `List`, recorded in the typed tree as `ForIteration::Range` or `ForIteration::List { mutable }`. The MIR lowers both through one shape (a cursor, a test against the end, an element binding, an advance) in `lower_for`, so a third built-in iterable (a string's characters, the ROADMAP follow-up) adds a case, not a loop lowering.

A user-visible protocol is deliberately not designed now. Kotlin's convention (`iterator()`, `hasNext()`, `next()` as operator functions) would need an iterator value holding a borrow of its list, which needs struct fields of reference type, i.e. lifetimes, against the no-lifetime-annotations rule, and a `next()` returning a borrow tied to the iterator, the "lending iterator" problem. Internal iteration (`for` as a call of `forEach` with a closure) cannot carry `break`, `continue` or `return` across the closure. A cursor convention (`start()`, `hasElement(c)`, `element(c): &T`, `next(c)` with a `Copy` cursor, borrows tied to the collection) would work today, but traits will define the protocol properly and this would have to be replaced. The protocol is a follow-up, with traits. Rejected: each of the three above.

### D11. Borrowing elements uses the existing rules (autonomous)

An index projection is owned by its list, as a box's dereference is owned by the box: `places.rs::extends_through_reference` is false for it, so dropping, moving or reassigning the list conflicts with element borrows, and an element borrow is a borrow of a part of the list, which any mutating member call conflicts with because it borrows the whole list mutably. Two index projections of one list always overlap (`places.rs` already says so), as in Rust: the checker does not compare index values. So `val first = &xs[0]; xs.add(4); println(first)` is `E5003`, holding `&mut xs[0]` and `&mut xs[1]` together is `E5003`, and returning `&xs[0]` from `fun first(xs: &List<String>)` is tied to `xs` by elision. Each access reads its index local, which `uses.rs` must record. The loop's borrow (`BorrowOrigin::Iteration`) is labelled "borrowed by this `for` loop" in diagnostics, with the next iteration as its later use.

Swapping two elements or borrowing two at once needs a member that does it safely (`swap(i, j)`, a follow-up). Rejected: comparing constant indices (`xs[0]` and `xs[1]` disjoint), which would make borrow checking depend on constant folding.

### D12. Equality where the element type has it (autonomous)

`==` on two `List<T>`, `contains` and `indexOf` are accepted exactly where `==` on `T` is: on concrete types, and rejected with `E4002` when `T` mentions a type parameter, the rule for `==` on a type parameter (`generics`, D5), so no bound has to be invented before traits. Lists are equal when their sizes are and their elements are pairwise, compared in order with `==` of `T` (structural for structs, by text for strings, by elements for nested lists). Rejected: an `Equatable`-like bound now (it would become a trait later and break every body using it, the reason `generics` left `==` out); identity comparison (meaningless for owned values).

### D13. A list is `{ pointer, size, capacity }` (autonomous)

24 bytes, pointer-aligned: the elements' address, the size and the capacity, as Rust's `Vec` and Prism's `String` (which adds a character count). The size and capacity are `Int`-sized words, so `size` is a load. A list owns no memory when its capacity is 0 or its elements are zero-sized; its pointer is then the element alignment, non-null, as a box of a zero-sized value (`heap-boxes`, D3), and a list of zero-sized elements has the largest `Int` as its capacity and never allocates. Element `i` is at `i * stride`. The runtime mirrors the layout as `PrismList` (`#[repr(C)]`), as `PrismString` mirrors strings.

`List<T>?` keeps the general tagged layout (32 bytes): the pointer is never null, so a null niche is possible, but it is the first word of a memory value rather than a scalar, which `codegen-aggregates`' niche (D7) does not cover. It is recorded as a follow-up. Rejected: a header allocation holding size and capacity (an extra indirection per access); a niche now.

### D14. Doubling growth, exact construction, no shrinking (autonomous)

`add` or `insert` on a full list grows the capacity to twice its value, or to 4 from 0 (Rust's `RawVec` minimum for small elements), so `add` is amortized O(1) and a million adds reallocate about 18 times. `listOf(...)` and `clone()` allocate exactly the size (none for an empty list), since their size is known. Nothing shrinks: `removeAt`, `removeLast` and `clear` keep the buffer, as Kotlin's `ArrayList` and Rust's `Vec` do; `trimToSize`/`ensureCapacity` are follow-ups. Growth calls a new runtime `prism_rt_reallocate(pointer, old_size, new_size, alignment)` in `allocation.rs`, which reallocates through the global allocator and updates the allocation map when checking is on, so a grown buffer may extend in place. A byte size that overflows a word is "out of memory". Rejected: Kotlin's 1.5x growth (more reallocations for little memory saved at these sizes); allocate-copy-free on growth (never extends in place).

### D15. Fast paths inline, slow paths in the runtime, per-type work in glue (autonomous)

- **Inline in generated code** (`lower.rs`): `size`, `isEmpty`, `isNotEmpty`, `indices`, element addresses with their bounds check, `add` (compare size with capacity, call the runtime only to grow, store the element at `pointer + size * stride`, increment the size), `removeLast` (check, decrement, move the element out to the destination), and `listOf` (one allocation through `prism_rt_list_with_capacity`, then stores).
- **Runtime calls** (`crates/prism-runtime/src/list.rs`, type-erased by element size and alignment): `prism_rt_list_grow`, `prism_rt_list_open_gap` (for `insert`: check the index, grow, move the tail up, return the gap's address), `prism_rt_list_close_gap` (for `removeAt`, after the element was moved out), `prism_rt_list_reverse`, `prism_rt_list_free`, and the two errors.
- **Glue** (`glue.rs`, local to the unit): drop (`#drop(List<T>)`: drop each element with the element's glue, if it has any, then free), clone (`#clone(List<T>)`), equality (`#equals(List<T>)`) and index-of (`#indexOf(List<T>)`) loops, which are per element type because they call the element's glue or inline comparison. `clear` calls the drop loop without the free. All list glue checks the stack, as `heap-boxes` makes recursive glue do, since `List<Node>` glue reaches `Node` glue.

`add` inline is what makes a million adds take a few milliseconds; everything that moves many bytes is a runtime `memmove`. Rejected: everything in the runtime with drop-function pointers (an indirect call per element per drop, and `add` through a call); everything inline (code size for rarely used paths).

### D16. Typed tree and MIR: built-in calls, index places and a list loop

- **Typed tree.** `BuiltinFunction::ListOf` (no receiver, its elements consumed, type arguments `[T]`) and one variant per member, named as written (`size`, `indices`, `add`, ...), all with receivers. `StatementKind::For { variable, iterable, iteration, body }`, where the old `range` field is renamed `iterable` and `iteration` is `ForIteration::Range` or `ForIteration::List { mutable }`; for a list, `iterable` is a `Borrow` node with the new origin `BorrowOrigin::Iteration` (shared, or mutable for a `&mut` iterable, whose written borrow is then a reborrow). Elements are `Index` nodes. `TypedFile::validate` checks the built-ins' types, `Index` on lists, and the loop variable's type against the iteration.
- **MIR.** The built-ins are `Call` terminators of `Callee::Builtin` (`builtin listOf`, `builtin add`, ...), so allocation failure and index errors happen at terminators or at the place access. `Projection::Index` becomes valid on `List<T>` places (`projection_type`), still never moved out of (`validate.rs`). The list loop: the borrow into a hidden local `_l`, `_n = builtin size(&*_l)`, a skip when `_n == 0`, per iteration a fresh scope binding `x = &(*_l)[_i]` (or `&mut`), the body, and a latch `_i = _i + 1; if _i == _n exit`. `_i` stays below `_n`, so the increment never overflows; the per-element bounds check always passes and is left for a later optimization (Tier 2 removes it).

### D17. Higher-order helpers belong to the bundled library (autonomous)

`forEach`, `map`, `filter` and their relatives (`any`, `all`, `count`, `sumOf`, `joinToString`, `removeAll`, `retainAll`) are not part of this change. Each is a few lines of Prism over this change's members:

```
public fun <T> List<T>.forEach(action: (&T) -> Unit) { for (x in this) action(x) }
public fun <T, R> List<T>.map(transform: (&T) -> R): List<R> { var out = listOf<R>(); for (x in this) out.add(transform(x)); return out }
public consuming fun <T> List<T>.filter(predicate: (&T) -> Bool): List<T> {
    var source = this; source.reverse(); var kept = listOf<T>()
    while (source.isNotEmpty()) { val x = source.removeLast(); if (predicate(x)) kept.add(x) }
    return kept
}
```

so they belong in the bundled library of `standard-library` (item 26), where they are generic code that `monomorphization` instantiates, `prism fmt` formats and `hot-reload` reaches, rather than MIR expansions the compiler would carry and later delete. `filter` consumes its list (keeping elements without cloning them, against the explicit-clone rule of `ownership`), and its predicate borrows. This packet adds that scope to item 26 in `ROADMAP.md`. Until then programs write the loops, or their own extensions on `List<T>`, which the bundled ones will not conflict with (a file's own extension shadows an imported one).

One consequence for compose: a closure passed to `forEach` is not a composable context, so composables are emitted from `for` loops, as §5 plans (`for (todo in todos.value) { TodoRow(todo) }`); Kotlin needs `inline` for its `forEach` to allow it.

Rejected: built-in helpers lowered as loops in MIR (throwaway once the library exists, and `filter` needs internal partial moves or the `reverse` trick in MIR); a borrowing `filter` that clones kept elements (a hidden clone).

### D18. A `collections` capability, plus stage deltas (autonomous)

The language-level rules (type, construction, members, elements, iteration typing, equality, borrows) form a new `collections` capability, as `patterns` was for `payload-patterns`. Runtime meaning goes to `execution`, representation to `code-generation`, and the tree and MIR shapes to `typed-tree` and `mir`, as for every construct. `loops` "For Loops Over Ranges" is modified to admit lists. Rules of other specs that lists inherit unchanged (receivers, copy-out, `E4017`, markers of generic types) are restated in `collections` only where a scenario pins them for lists, and the diagnostics catalog rows are widened directly. Rejected: spreading the language rules over `type-system`, `references` and `ownership` (each would need MODIFIED blocks over requirements that pending changes also modify).

### D19. A reported benchmark for adds (autonomous)

An ignored release test beside `run_timing.rs` runs `prism run` on a program adding a million `Int`s and summing them, reporting the median and 90th percentile wall time against 20 ms and failing only with `PRISM_ASSERT_TIMING=1`, like the startup benchmark (`codegen-core`, D22). A debug-build executable fixture runs the same program under `--check-allocations` for correctness. The estimate is a few milliseconds of adds and sums plus about 2 ms of startup. Rejected: asserting the time in `cargo test` (flaky on shared machines).

## Risks / Trade-offs

- **`&T` loop variables show through inference.** `val y = x` makes `y` a `&Int`, which reads the same everywhere, and declared types (fields, parameters, `ys.add(x)` on a `List<Int>`) copy out. The visible case is `listOf(x)` in a loop over `Int`s, which infers `List<&Int>` and is `E4022`; its help suggests `listOf<Int>(x)` (a task).
- **Bounds checks in loops cost a compare per element in Tier 1.** Acceptable for Tier 1; the follow-up removes checks the loop proves.
- **Conservative element overlap** rejects some correct programs (two mutable element borrows). `swap` and later split borrows are follow-ups.
- **Indexing a `List<T>?`** needs `?.`: `xs?.get` does not exist, and `xs?[0]` is not syntax. Programs unwrap first (`xs?.size`, `if (xs != null) xs[0]`); safe indexing is a follow-up.
- **No consuming iteration** makes moving all elements out O(n) only through `reverse()` and `removeLast()`, which the library's `filter` uses. A consuming loop form is a follow-up.
- **Built-in members become declarations later** (D1, D6). If `standard-library` lets a file's own extension shadow prelude members, today's `E4010` for an extension named `add` on `List` becomes acceptance: a relaxation, not a break.

## Migration Plan

No user code changes: `List` and `listOf` were unknown names. Internally, `StatementKind::For`'s `range` field becomes `iterable` with an `iteration`, which every match over it (type checker, typed-tree dump and validation, MIR builder, formatter-independent) must follow, and the typed-tree and MIR snapshots of `for` loops are re-blessed with the new dump fields.

## Open Questions

None blocking. `standard-library` decides whether the list members move into a prelude package (`/prism/collections` or a core prelude) and whether prelude members may be shadowed.
