# Design: heap-boxes

## Context

See `proposal.md` for motivation. This is item 17 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17, and the deferred item "`Heap<T>` construction and unboxing". `runtime-strings-and-drops` recommended a design (its D19), which this change follows except where noted. The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

This change builds on `codegen-aggregates` (layouts, the calling convention, `Heap<T>` laid out as a pointer, the `Heap<T>?` null niche of its D7), `runtime-strings-and-drops` (the allocator `prism_rt_allocate`/`prism_rt_free`, per-unit drop and clone glue in `crates/prism-codegen/src/glue.rs`, `--check-allocations`) and `payload-patterns` (patterns that bind payloads, `crates/prism-typechecker/src/patterns.rs` and `exhaustiveness.rs`, `crates/prism-mir/src/build/pattern.rs`), all three archived. The deltas were written before the last two were archived and before the main specs were shortened, and were re-based on the main specs at implementation time (see "Reconciliation with the main specs" below).

Current state (after those changes):

- **The type exists.** `Type::Heap(inner)` is in the type checker, the typed tree, MIR and the ABI. `Heap<T>` is never `Copy`, and it is `Clone` exactly when `T` is. `E4020` and `E4022` cover arity and borrows in its argument. The parser rejects a `Heap<...>` extension receiver with `E2001`. `E4023` accepts recursion through a box.
- **Some reads look through boxes.** Field access (`h.x`, `peel_indirection_nodes`), extension receivers in every mode (the `Layer::Heap` of `infer_method_call`) and `s.length` look through any `Heap<T>` layers. Field writes through a box follow the place's mutability (`h.x = 3` needs `var h`, and `contents_writable`). Operators peel references only (`peel_reference_nodes`), so `h + 1` is `E4002`. Copy-out (`copy_out`) peels references only, so `val n: Int = h` is `E4005`/`E4002`. A box passed to a `&T` parameter is `E4004` (`implicit_borrow` does not look through it).
- **No construction, no write, no move-out.** `Heap(1)` is `E4001`. `*h = v` is `E4019` (`assignment_target`). A `consuming fun` receiver through a box is `E4017`, "moving out of a 'Heap' is not supported yet" (`pass_receiver`). Moving a field through a box is `E4017` ("Moves Take Whole Values Only").
- **Covariance.** `is_assignable_to` (`crates/prism-typechecker/src/types.rs`) accepts `Heap<U>` as `Heap<T>` when `U` is assignable to `T`, and the typed tree marks it as a `Subtype` step that changes representation (`Heap<Int>` given for `Heap<Int?>`). `codegen-core` rejects it with `E6001`.
- **MIR and the borrow checker.** `Projection::Deref` of a `Heap<T>` place is the contents (`projection_type`). The validator rejects any move out of a dereference. `places.rs` treats a box's dereference as memory the box owns (`extends_through_reference` is false for it), so `&mut m.x` conflicts with `m`, and returning `&h.x` from a by-value parameter is `E5008`. Today's `prism check` already shows both.
- **Backend.** `Heap<T>` has a pointer layout, and `Heap<T>?` is one pointer with `null` as 0. `supported.rs` rejects every `Heap<T>` value as `Construct::HeapValues`, naming `heap-boxes`. Equality is expanded inline per operator (`compare` in `lower.rs`, `codegen-aggregates` D13) and follows the pointer of a box. That expansion would not terminate for a type that reaches itself through boxes. `runtime-strings-and-drops` D11 leaves equality glue to this change, and its D9 the stack check of recursive glue.
- **Patterns.** `payload-patterns` lists patterns through `Heap<T>` as a non-goal waiting for this change: a `Heap<T>` part is matched only by `_`, a binding or a value.

## Goals / Non-Goals

**Goals:**
- Recursive data (expression trees, linked lists, trees) can be built, read, matched, updated, taken apart, cloned, compared and dropped, with every allocation freed exactly once, which `--check-allocations` checks in every fixture.
- A box reads like a reference to its contents everywhere a reference reads implicitly, so code over `Heap<T>` looks like code over `&T`.
- Moving contents out is explicit (`unbox()`), except at a consuming receiver, where receivers already look through boxes implicitly.
- No layout-changing subtyping through boxes, and no new MIR place or rvalue kind: the box is a pointer, its contents a `Deref` place, and construction and unboxing are calls.

**Non-Goals:**
- Iterative drop of long chains, a read form of `*`, implicit unboxing at by-value positions, extensions on `Heap<...>` receivers, placement construction, other niches, shared or reference-counted boxes (D14, D20).
- Generic bodies over `Heap<T>` in the backend, which wait for `monomorphization` like every generic body.

## Decisions

### D1. Prerequisites: patterns first (autonomous)

The use cases need `payload-patterns`. An expression tree cannot be evaluated without reading a variant's payloads (`Expr.Add(val l, val r)`), which only patterns do. Patterns also have to decide what they do at a `Heap<T>` part (D9). Struct-based lists and trees could be read without patterns (`n.next?.sum() ?: 0` through a safe call), but enums could only be built, compared, cloned and dropped. Running half the use cases now and adding deref patterns in a later change would leave `payload-patterns`' non-goal open twice. So `payload-patterns` (item 21, already proposed, depending only on items 15 and 16) lands before this change, and this change includes patterns through boxes. `ROADMAP.md` has to be reordered so that `payload-patterns` comes right after `runtime-strings-and-drops`. This packet does not edit the roadmap. The orchestrator does, logging it as a process decision. Rejected: boxes before patterns, with recursive enums tested only through construction, cloning and drops, and deref patterns in a follow-up change; and implementing the pattern part (task group 7) last, conditionally, which would leave this change's `patterns` delta unimplementable on its own.

### D2. Construction is a built-in constructor, `Heap(value)` (autonomous)

`Heap(value)` is written like a struct constructor and is a call of the new built-in `HeapAllocate` (dump name `Heap`). The contents' type `T` comes from an explicit type argument (`Heap<Int?>(1)`, which reuses the call type-argument syntax of `generics`), else from the expected type (`val h: Heap<Int?> = Heap(1)`, a `Heap<Expr>` parameter), else from the argument. The argument is checked against `T` in a consuming position, so `Heap(u.name)` is `E4017` and `1` is wrapped into `Int?` before it is boxed. That conversion is how a `Heap<Int?>` gets built from an `Int` once boxes are invariant (D8). Exactly one positional argument is allowed (`E4004` otherwise, as for the output built-ins). `E4020` and `E4022` apply to the type argument as they do to annotations.

`Heap` resolves like a type name in call position, which local values cannot hide (the `type-system` rationale "Type lookup skips values"), so `Heap(...)` always constructs a box. A top-level `fun`, `struct`, `enum` or `val` named `Heap` could never be called or named, so it is `E4010` (`DuplicateSymbol`), the way a type parameter named `Heap` already is. `Heap` named without a call is `E4001`, as `println` is.

Rejected: a free function `box(v)` (one more name, and Kotlin and Swift users read `Heap(v)` as the type's constructor); Rust's `Box::new(v)` (Prism has no associated functions); a `heap` keyword or `new` operator (new grammar for one call); letting user declarations shadow it like `print` (a struct named `Heap` would then construct something that is not a `Heap`).

### D3. A box is a pointer to an exactly sized allocation (autonomous)

A `Heap<T>` is a non-null, `T`-aligned pointer to an allocation of exactly `size_of(T)` bytes with `T`'s alignment, made by `prism_rt_allocate` and freed by `prism_rt_free` with the same size and alignment (`runtime-strings-and-drops` D8). Construction evaluates the argument first and allocates after, as a call evaluates its arguments before it runs: `Heap(make())` runs `make` before the allocation can fail. Then it moves the value into the allocation, as a bitwise copy (`codegen-aggregates` D12). A failed allocation is the runtime error "out of memory", from the runtime, with no location (`runtime-strings-and-drops` D17).

A **zero-sized** `T` (`Unit`, a fieldless struct, an enum whose layout is empty) allocates nothing. The box is the address equal to `T`'s alignment, which is non-null and aligned, as Rust's `NonNull::dangling`. Freeing it frees nothing, and nothing is read or written through it. The allocation check then sees no allocation, which a fixture asserts. The **null-pointer niche** of `codegen-aggregates` D7 is unchanged: `Heap<T>?` is one pointer and `null` is 0, which no box is, zero-sized ones included.

Rejected: a header word (a size or a count) before the contents (nothing reads it, and a count would be the hidden reference counting the zero-cost memory safety invariant forbids); allocating one byte for zero-sized contents (an allocation per box of `Unit` for nothing); allocating first and evaluating the argument into the box (placement). Placement saves a copy of large values, but it reorders "out of memory" before the argument's own effects, and it is an optimization the backend can make later where the argument cannot fail or print. It is recorded as a follow-up.

### D4. A box reads like a reference to its contents (autonomous)

Every place where a reference is read implicitly reads through `Heap<T>` layers too, at any depth and mixed with references:

| Read | Through `&T` (today) | Through `Heap<T>` (now) |
|---|---|---|
| Fields, receivers, `length` | yes | yes (already) |
| Operators, `if`/`while`/subjectless `when` conditions | yes | yes |
| Copy-out of a `Copy` value where a `T` is expected | yes | yes |
| `print`/`println` arguments, scalar `toString()` | yes | yes |
| A `T` place given for a `&T` parameter | borrowed where it lives | a `Heap<T>` lends its contents |
| `&(T?)` as `(&T)?` | yes | `&Heap<T>` as `&T`, `&mut Heap<T>` as `&mut T` or `&T`, `Heap<T>?` as `(&T)?` |

- **Lending the contents.** Where an argument's parameter is `&T` and the argument is a `Heap<T>`, possibly behind more boxes and references, the argument lends its contents: a place is read in place and its contents borrowed (`&*h`), and a temporary box is borrowed at its contents and dropped at the end of the statement, like any argument temporary. `eval(h)`, `eval(l)` for `l: &Heap<Expr>`, and `eval(Heap(e))` are all accepted.
- **The box view.** `&Heap<T>` converts to `&T`, and `&mut Heap<T>` to `&mut T` or `&T`, wherever such a type is expected (initializers, returns, arguments), as `&(T?)` converts to `(&T)?` (`reference-access` D8). It is a reborrow of the contents, `&**r`. At run time it loads the box pointer once, which is also the contents' address. `Heap<T>?` converts to `(&T)?`, a borrowed `Heap<T>?` likewise, and a `Heap<T>?` place passed to a `(&T)?` parameter lends its contents. With the niche, the box pointer and the contents pointer are the same bits, so this costs nothing. So a tree's `t.left` (a `Heap<Tree>?`) can be passed to `fun depth(t: (&Tree)?)`.
- **Exact types first.** These conversions apply only where the value does not already fit. A generic parameter `&T` given a `Heap<Int>` infers `T = Heap<Int>` and borrows the box, as inference matches exactly before any adjustment (`generics`).

The rule is that a box is "an owning `&mut`": it reads like one, and it is written through like one when its place is mutable (D5). Rejected: reading only through fields and receivers, as today (a `Heap<Int>` could not be read at all without moving it out, and `eval(h)` would need an explicit borrow syntax that `*`, writes-only, does not give); Rust's `Deref` coercion as a general trait mechanism (no traits yet; these are fixed rules for one built-in type); a read form of `*h` (`reference-access` D5 made `*` writes-only, and reads are implicit everywhere else).

### D5. `*h = v` writes the contents of a mutable box (autonomous)

`*h = v` and `*h op= v` write a box's contents: the old contents are dropped and `v` is moved in, and the allocation is kept (no free and no allocation). `*` peels exactly one layer, a reference or a box, so `**r = v` writes the contents of a box reached through `r: &mut Heap<T>`, while `*r = Heap(v)` replaces the whole box (dropping, and so freeing, the old one). Each layer written through must be writable. A reference layer must be `&mut`, as today. A box layer must be a mutable place: a `var` binding, a `var` field reached by a writable path, or a box reached through `&mut`. This is the rule that field writes through a box already follow (`contents_writable`), and it differs from references, where a `val` binding of a `&mut` may be written through. A box owns its contents, so its contents are as mutable as its place, as Rust's `let mut b: Box<T>` is. Writing through a box that is not writable is `E4003`, with help saying to declare the binding (or field) `var`. `*x` on a value that is neither a reference nor a box stays `E4019`, whose message now says "`*` needs a reference or a `Heap`".

`h = 5` for a `var h: Heap<Int>` assigns an `Int` to a box: `E4002`, with help suggesting `*h = 5`, as `reference-access` D6 helps for `r = 5`.

Rejected: writes through a `val` box (the contents would be mutable while the binding is not, unlike fields); D19's `*r = v` semantics without the one-layer rule (`*r` for `r: &mut Heap<Int>` would be ambiguous between replacing the box and writing its contents).

### D6. Moving out: the consuming built-in `h.unbox()` (autonomous)

`h.unbox(): T` is a built-in member of `Heap<T>` with a consuming receiver. It moves the contents out and frees the box (without dropping the contents). It follows the move rules of every consuming receiver: a whole owned local or parameter, or a temporary (`Heap(1).unbox()`), is moved. A field (`n.next.unbox()` for a `val next: Heap<Node>`) or a top-level `val` is `E4017`, and so is a box reached through a reference, "borrow `r`", with help suggesting `.clone().unbox()` for a `Clone` content. `n?.unbox()` on an owned `Heap<T>?` gives a `T?`, as consuming safe calls do. It takes no arguments (`E4004`). It applies to the outermost box after references are peeled, so `hh.unbox()` on a `Heap<Heap<Int>>` gives a `Heap<Int>`. It comes after fields and the built-in `clone()` and before extensions, so an extension `fun Node.unbox()` is not reached through a box (call it on a borrow of the contents, `val n: &Node = h; n.unbox()`). That extension is still callable on a `Node`, so it is not `E4010`.

A non-`Copy` content where a value of type `T` is expected (`val u: User = h`, `take(h)` for `fun take(u: User)`, `return h`) is `E4017` with the source "the contents of `Heap` `h`" and help suggesting `h.unbox()`. This mirrors "Moving Out of a Borrow Is Rejected", and a `Copy` content is copied out instead (D4).

Rejected: Rust's move out of `*b` (Prism has no read `*`); implicit unboxing at every by-value position (a type mismatch would silently free a box, and `val x: T = h` would read differently from `val x = h`, which moves the box); a free function `unbox(h)`; the names `take()` (reads as "borrow" in UI code), `get()` (Kotlin's non-consuming getter) and `into()`.

### D7. A `consuming fun` receiver reached through boxes is unboxed (autonomous)

`h.toAccount()` for `consuming fun User.toAccount()` and an owned `h: Heap<User>` moves `h`, unboxes it (freeing the box) and passes the contents, once per box layer when the receiver is behind several (`Heap<Heap<User>>`). This turns `E4017` into acceptance. The receiver must be movable as a whole: a field or top-level `val` holding the box stays `E4017`, as for any consuming receiver. Any reference layer makes it a move out of a borrow, `E4017` as today, and a `Copy` receiver through references and boxes is still copied out, leaving the box alone.

Receivers look through boxes in every other mode with nothing written (`extension-methods` D8), and Rust's method calls move out of a `Box` the same way. Requiring `h.unbox().toAccount()` would make `consuming` the one mode that does not, for no safety gain: the source is consumed either way and the borrow checker sees one move of `h`. D19 of `runtime-strings-and-drops` suggested keeping `E4017` and writing `h.unbox().f()`. That stays possible. Rejected: keeping `E4017` (above); unboxing through references (a move out of a borrow).

### D8. Boxes are invariant (autonomous)

`Heap<U>` is assignable to `Heap<T>` only when `U` and `T` are the same type, up to earlier errors. This is the rule of references (`reference-access` D9) and of generic types (`generics` D11), and it removes the covariance rule from `is_assignable_to`. `Heap<Int>` given for `Heap<Int?>` changes the layout of the contents (8 bytes versus 16), so covariance would need a re-allocating adapter hidden behind an assignment. The type checker reports `E4002`, `E4004` or `E4005` as for any mismatch, with help saying that boxes are invariant and that the contents can be converted when the box is built (`Heap<Int?>(...)`). The `Nothing` rule is unaffected (`return` in a `Heap<T>` position). Joins follow: `if (c) Heap(1) else Heap(null)` has no join without an expected type, while `val h: Heap<Int?> = if (c) Heap(1) else Heap(null)` checks each branch against `Heap<Int?>`.

After this, the typed tree's only representation-changing `Subtype` step is function-type variance through nullable parameters (the `ROADMAP.md` risk from `typed-tree` D6), which `closures` must handle. Rejected: covariance with a re-boxing adapter (a hidden allocation and free per conversion); covariance only for representation-preserving steps (`&mut` to `&` cannot occur in a box, borrows being `E4022`, so there is nothing left to allow).

### D9. Patterns look through boxes; bindings take the box (autonomous)

On top of `payload-patterns`:

- **Matching contents.** Wherever a constructor, value or range pattern is checked against a part, or a subject, of type `Heap<T>` (behind any boxes and references), it is checked against `T` and matches the contents. So `Expr.Add(Expr.Lit(0), val r)` and `when (h) { Expr.Lit(val n) -> n; ... }` for `h: Heap<Expr>` work. This is the auto-deref of field access applied to patterns. It needs no `box` pattern syntax, which Rust has needed for years and still lacks on stable.
- **Bindings and wildcards take the box itself.** `val l` at a `Heap<Expr>` payload binds the part as typed, with the subject's mode: a `Heap<Expr>` moved out of an owned subject, a `&Heap<Expr>` under `&`, or a `&mut Heap<Expr>` under `&mut`. A part is always what the declaration says, and a binding that moved the contents out would hide an unboxing. Recursion stays natural: `eval(l)` lends `l`'s contents (D4).
- **Taking apart through a box unboxes it.** When a by-value non-`Copy` binding sits inside the contents of a box part (`Expr.Neg(Expr.Lit(val s))` over an owned `Expr` holding strings), taking the subject apart moves the box part out, unboxes it, freeing the box, and takes the contents apart. Unbound parts are dropped where the branch ends (`payload-patterns` D8). So no partial move ever crosses a box, and drop elaboration still works on whole locals.
- **Exhaustiveness.** A `Heap<T>` column is the column of its contents: a box has exactly one constructor, so the usefulness algorithm (`payload-patterns` D11) looks through it, and witnesses are printed without any box syntax (`Expr.Add(Expr.Lit(_), _)`). Patterns are finite, so the algorithm still terminates.

Rejected: an explicit deref pattern (`box p`, Rust's unstable syntax), which is noise where field access is already implicit; bindings that take the contents (`val n` at a `Heap<Int>` binding an `Int`), which would make the binding's type differ from the payload's declared type and, in a consuming subject, free the box silently; leaving deref patterns out (simplifiers like `Add(Lit(0), e) -> e` would need nested `when`s).

### D10. Typed tree and MIR: two built-in calls, no new place or rvalue

- **Typed tree.** `BuiltinFunction::HeapAllocate` (dump `Heap`, no receiver, the argument consumed, the call's type arguments `[T]`) and `BuiltinFunction::Unbox` (dump `unbox`, `has_receiver` true, the receiver moved). A consuming receiver through boxes (D7) is the extension call with its receiver argument wrapped in one `unbox` call per box layer around the move of the owner. Reading through a box is a `Deref` node, so copy-out is a copy of a dereference, lending is a borrow of a dereference with origin argument, and the box view is a borrow of a dereference with origin conversion. The removed covariance means no `Subtype` node ever has `Heap` types. `TypedFile::validate` checks the built-ins' types: `HeapAllocate` takes a `T` and has type `Heap<T>`, and `Unbox` takes a moved `Heap<T>` and has type `T`.
- **MIR.** Both are `Call` terminators of `Callee::Builtin`, printed `builtin Heap` and `builtin unbox`, so allocation failure, like any call, happens at a terminator. `unbox` has a receiver, and its operand is a `Move` of the box. A box's contents are the existing `Deref` projection of its place. Writing them is an assignment to `(*_1)`, which already drops the old value first ("Assignments Drop the Old Value"). Moves out of a dereference stay invalid (`validate.rs`), so contents leave a box only through `unbox`. The validator checks the operand and destination types of both built-ins.
- **Patterns.** The typed tree records a pattern of a box's contents as `Pattern::Deref(inner)`, beside `Pattern::NonNull` (implementation: a pattern has no place of its own to put a `Deref` step on, so the box layer is a pattern node, as a nullable's is). In MIR (`build/pattern.rs`), tests recurse into `Deref` places at box parts. Taking apart through a box (D9) moves the box part into a temporary, as a nested part already is, calls `builtin unbox` on it into a second temporary, and destructures that. The guard, as before, reads the parts of moving bindings in place, here through the box's `Deref`.

Rejected: an `Rvalue::Box` (Rust's old `box` expression) and a move out of `(*_1)` permitted for boxes only (Rust's special case). Both would put allocation or partial moves of boxes into every analysis, while calls are already handled everywhere.

### D11. The borrow checker treats a box's contents as owned by the box

This is already true and becomes observable: `*h` and `h.x` are places under `h` with a non-reference `Deref`, so a borrow of them conflicts with uses of `h` by prefix (`E5003`). Assigning or moving `h` while they are borrowed is `E5004` or `E5005`, and `h` ending while they are borrowed is `E5006`. Returning a borrow of a by-value box's contents is `E5008`, while a borrow of the contents of `r: &Heap<T>` is tied to `r` and may be returned. `*h = v` reads `h` and needs it initialized (`E5001` after `h` moved). `unbox` and `Heap` are ordinary calls for moves and loans (`uses.rs`, `loan_flow.rs`): `Heap(v)` moves `v`, and `h.unbox()` moves `h`.

**Boxing a capturing closure is an escape (autonomous):** `Heap({ greet(s) })` is `E5007`, labelled "stored in a box here", like storing the closure in a struct field (`escapes.rs`), because a box is an owned container that can be returned or stored anywhere. `closures` revisits both together when it designs escaping closures. Rejected: letting boxes carry the closure's loans (`carries_loans` already says a `Heap<() -> Int>` could), which would make a box of a closure a borrowing value unlike every struct.

### D12. Box glue: contents first, then the allocation

Following `runtime-strings-and-drops` D9 and D10, `Heap<T>` is one more type that owns storage, with glue functions of its own in the unit's `GlueTable`, named after the box type:

- `<module>#drop(Heap<T>)` loads the pointer, runs `T`'s drop glue on it when `T` owns storage, then frees the allocation (nothing for a zero-sized `T`). The contents are dropped before the memory they live in is freed.
- `<module>#clone(Heap<T>)` allocates a new box (or makes the dangling pointer for a zero-sized `T`), then clones the contents into it by `T`'s clone glue, or copies them when `T` owns nothing, and writes the new pointer to the destination.
- `unbox` is lowered inline: it copies the contents to the destination and frees the allocation, without dropping. Construction is inline too: `prism_rt_allocate` with `T`'s size and alignment (skipped for a zero-sized `T`), then the operand written to the allocation.

A struct, enum or nullable holding a box calls the box's glue for that part; a `Heap<T>?` is one pointer, so its glue tests it for 0 rather than reading a tag. Glue takes the value's address, so a box held in a Cranelift variable is spilled to a stack slot before its drop glue is called, and its clone is written to a slot and read back (implementation). A recursive type's glue is recursive: `#drop(main::Expr)` calls `#drop(Heap<main::Expr>)` for an `Add`'s payloads, which calls `#drop(main::Expr)`.

### D13. Equality of boxes compares contents through equality glue (autonomous)

`==` on two boxes compares their contents. It is structural, as for every type (`codegen-aggregates` D13), and the address is never compared. Inline expansion would not terminate on a type that reaches itself through boxes, so it stops at box layers: comparing the contents of two boxes whose contents are a struct, an enum or a nullable of one calls the equality glue `<module>#equals(<type>)` of the contents' type, which takes the two addresses and returns a `Bool` (an `i8`). Its body is the same inline expansion, stopping at its own box layers. Contents of other types (scalars, strings, nullable scalars) are compared inline, since they cannot recurse. Equality glue lives in the `GlueTable` beside drop and clone glue, local to the unit, named and printed like them. (Implementation: the inline expansion moved from `lower.rs` to `crates/prism-codegen/src/equality.rs`, over a function builder and the unit's context, so that bodies and equality glue share it.)

There is no pointer-equality shortcut: two boxes at the same address are both the same box, which only `&h == &h` can compare, but a `Double` content that is `NaN` must still be unequal to itself. Rejected: equality glue for every aggregate type (changes `codegen-aggregates`' inline code everywhere for nothing); glue only for types that reach themselves (a reachability analysis to save a call per box, which the inline expansion cannot avoid anyway once a box is reached); a pointer shortcut for types without `Double` (an optimization, not a semantics).

### D14. Glue checks the stack; deep structures are a stack overflow (autonomous)

Every drop, clone and equality glue function checks the stack on entry like a compiled body (`codegen-core` D27), so dropping, cloning or comparing a structure deeper than the 8 MiB program stack allows raises "stack overflow" instead of crashing the process. Glue has no declaration to point at, so this error writes no location line, like "out of memory". The `execution` requirements on runtime errors and stack overflow gain that exception. The check is one load and one compare per glue call. Checking only glue that can recurse would save it elsewhere, but every glue reached from a box can recurse in general, and the rule is simpler uniform.

A linked list of about a hundred thousand nodes or more is dropped recursively and overflows (Rust's `Box` lists have the same limit). Iterative drop is a follow-up: for a type whose glue recurses into one box part, drop that part last as a loop, by tail calls (Cranelift's `return_call`) or a loop over the box chain. It changes no program's meaning, only which programs overflow. Rejected: iterative drop now (it is an optimization over the glue this change defines, and it needs the "last box part" analysis); no stack check (a segmentation fault without output); reporting the location of the drop (every glue call would pass a location).

### D15. Diagnostics: no new codes

Every error reuses a code, widened where needed, with the catalog rows edited directly: `E4001` (`Heap` without a call), `E4002`/`E4004`/`E4005` (invariance and mismatches, with help), `E4003` (writing through a box that is not writable), `E4004` (arguments of `Heap` and `unbox`), `E4010` (top-level declarations named `Heap`, row widened), `E4017` (moving a box's contents, `unbox` and consuming receivers through references, row widened), `E4019` (`*` on a value that is neither a reference nor a box, row widened), `E4020`/`E4022` (the type argument of `Heap(...)`) and `E5007` (boxing a capturing closure, row widened). `E6001` stops naming `'Heap' values`.

### D16. No new capability (autonomous)

Boxes extend the capabilities they touch, as strings did (`runtime-strings-and-drops` D21): construction, invariance and operators in `type-system`, reading and writing in `references` (a box reads and is written like an owning reference), moving out in `ownership`, precedence in `extension-functions`, patterns in `patterns`, built-ins in `typed-tree` and `mir`, representation and glue in `code-generation`, and runtime meaning in `execution`. Rejected: a `heap` capability, which would split each rule from the reference and move rules it mirrors.

### D17. Existing tests and the `E6001` fixtures

- The `ownership` fixture `consuming-through-a-borrow--heap` flips from `E4017` to accepted (D7). The `type-system` fixtures of `Heap` covariance, if any, and the typechecker unit tests of `Heap` assignability (`checker.rs` tests using `Type::Heap`) flip to invariance (D8).
- The `code-generation` fixtures and support tests that pinned `Heap<T>` as unsupported (`an-enum-holding-a-heap-value-is-rejected`, `unsupported-constructs-in-several-forms--heap-nullable`, `a-struct-holding-a-heap-value`, `a-struct-is-rejected`, `one-error-per-body`, `dumping-a-file-the-backend-rejects`, and `every_row_of_the_table` in `supported.rs`) move to function values (`struct Task { val run: () -> Unit }`, `fun first(f: () -> Int): Int = 1`), which `closures` will support. `an-enum-holding-a-heap-value-is-rejected` becomes a running program.
- The `code-generation` requirement "Unsupported Constructs Are Rejected Before Code Generation" is replaced, with the same rule, by "Constructs the Backend Lacks Are Rejected Before Code Generation" (REMOVED and ADDED in the delta), the name `monomorphization` and `closures` modify. "Aggregates Holding Heap or Function Values Are Rejected" is replaced by "Aggregates Holding Function Values Are Rejected": its scenario "An enum holding a heap value is rejected" no longer holds, and OpenSpec does not let a MODIFIED block drop a scenario. "A struct holding a heap value" becomes a running program under "Box Glue Drops the Contents Before Freeing".
- `classify` in `supported.rs` now walks into a box's contents to find what they need (a `Heap<() -> Unit>` is still rejected for function values), with a set of visited types, since contents may reach the type being classified.

### D18. Allocation failure and zero-sized boxes are tested in Rust

No Prism program can portably make a box allocation fail, since no type is large enough, so "out of memory" from `Heap(...)` is covered by the runtime's own tests of `prism_rt_allocate` (`runtime-strings-and-drops`), which every box allocation calls. Zero-sized boxes are an executable fixture, where no live allocation is reported, and a codegen test reading `allocation_statistics()`, which shows that no allocation was made.

### D19. Deferred, as `ROADMAP.md` items on archive

Iterative drop of long chains (D14); placement construction (D3); `Heap<...>` extension receivers (the existing item); a read form of `*`, if one is ever wanted (the `reference-access` item "a syntax for reborrowing a `&mut` into a local" covers it); implicit unboxing at by-value positions, if programs show the need (D6); niches for other types (the existing `codegen-aggregates` item).

### D20. Non-goal: shared boxes

`Heap<T>` has a single owner. Sharing (`Rc`, `Arc`) is hidden or explicit reference counting, which the zero-cost memory safety invariant excludes. Graphs use typed handles (`docs/architecture.md` §3). Nothing in this change prepares for sharing.

### D21. Owned boxes lend only to reference parameters (autonomous, at implementation)

D4 says that "`Heap<T>?` converts to `(&T)?`". Read literally, an owned box would then be borrowed implicitly in any position, while an owned `T` is borrowed implicitly only as an argument of a reference parameter (`reference-access` D1: `val r: &T = x` needs `&x`). So an owned `Heap<T>` or `Heap<T>?` lends its contents only to a shared `&T` or `(&T)?` parameter, from where it lives or from a temporary, and never to a `&mut T` parameter (a mutable borrow is never implicit). A borrowed box (`&Heap<T>`, `&mut Heap<T>`, `&(Heap<T>?)`) converts wherever such a type is expected, mutably only through `&mut` layers. `val r: &T = h` is `E4002`, with help suggesting `&h`. Rejected: implicit borrows of owned boxes in value positions, which would make boxes the one owned value borrowed without `&`.

### Scenario fixes at implementation

- "Wrong arguments of Heap rejected": `Heap<Int>("x")` is an argument that does not fit its parameter, so it is `E4004` like the other three calls, not `E4002`.
- "Measuring a tree" passed `t.right?.left` to a `(&Tree)?` parameter. A safe navigation is a temporary, so that argument moves the field out of the borrow (`E4017`), for boxes as for any `T?` (`f(w.u?.n)` for `f(s: (&String)?)` is rejected the same way today). Lending a chain of fields read in place is not part of this change; the scenario now reads `t.left?.left?.value` and `t.right?.left?.value` instead, and the limitation is a `ROADMAP.md` follow-up on archive.
- "Box built-ins are calls" said `builtin Heap` takes "a copy of `n`": MIR passes the value of `n` through an argument temporary, and prints the type argument, `builtin Heap<Int>`.

### Reconciliation with the main specs (2026-10-18, at implementation)

The packet was proposed before `runtime-strings-and-drops` and `payload-patterns` were archived and before the main specs were shortened (8e5cc47). Its deltas were re-based so that each expresses only this change's edits against the current main text, and archiving restores no long text:

- `code-generation`: "Drops of Values That Own Nothing Do Nothing" no longer exists (its parts went to "Drops Run Drop Glue" and the `E6001` requirements). Its MODIFIED block is gone; "A struct holding a heap value" moves, as a running program, to the ADDED "Box Glue Drops the Contents Before Freeing", and the function-value scenario to the ADDED "Aggregates Holding Function Values Are Rejected", which replaces the REMOVED "Aggregates Holding Heap or Function Values Are Rejected" (it also gains "An enum holding a function value is rejected" in place of the heap one). The glue-naming sentence and "Glue of one type in two files" now live in "Glue Has No Function Table Slot", so that requirement is MODIFIED (adding `#equals`) instead of "Symbols Derive from Declaration Paths", which this change no longer touches. "Cranelift IR Dump" is re-based on the shortened text, adding only equality glue and the function-value scenario.
- `typed-tree`: "Built-In Operations Are Identified" was split into "Built-In Clone and Output Are Identified", "Built-In String and Scalar Members Are Identified" and "Ranges Are Range Nodes". The box built-ins become an ADDED "Built-In Box Operations Are Identified"; the first two are MODIFIED only to say that `Copy` values are copied out of boxes too, the second gaining "Built-in members through a box". "Ranges Are Range Nodes" is untouched.
- `mir`: "Calls Name Their Callee" was split; the receiver sentence and its scenarios are now "Receivers Lead the Operands of Calls". Each is MODIFIED with only `Heap`/`unbox` added, and "Box built-ins are calls" goes under the latter.
- The other MODIFIED blocks (`type-system`, `references`, `ownership`, `extension-functions`, `execution`) matched the main text apart from this change's edits and are unchanged.

Pending packets: `monomorphization` and `closures` MODIFY "Constructs the Backend Lacks Are Rejected Before Code Generation", whose name stays. `closures` removed "Aggregates Holding Heap or Function Values Are Rejected"; its REMOVED header now names "Aggregates Holding Function Values Are Rejected" (edited in that packet). `monomorphization` still MODIFIES "Symbols Derive from Declaration Paths" with the long pre-8e5cc47 text, including the glue sentence, and its "Glue Is Local to Its Code Unit" and "Cranelift IR Dump" blocks carry the long `dump-clif` wording; when it is applied it must move its glue-naming sentence to "Glue Has No Function Table Slot" and re-base those blocks. `closures`' "Cranelift IR Dump" needs the same re-basing. `escaping-closures` REMOVES "Capturing Closures Do Not Escape", which this change MODIFIES to cover boxes; its replacement must keep boxing a capturing closure covered. `string-templates` and `collections` touch none of these requirements.

### Findings at review (2026-10-18)

The review ran recursive enums, lists and trees (built in loops, evaluated, cloned, compared, written through `*h = v`, unboxed, taken apart through boxes, dropped at 10k and 100k nodes) under `--check-allocations` and found no leak, double free or wrong output; 300k nodes end with "stack overflow" (status 101), as D14 intends. It fixed two bugs as plain fixes, each with a scenario in the main specs:

- **Branch and `?:` operands read in place crashed MIR.** `"a" + (k.t ?: "-")`, `(k.t ?: "-").length`, `when (k.t ?: "-")` and `s += (if (c) u.name else "x")` were inferred in a reading position, so a non-`Copy` place in a branch was neither moved nor copied and MIR panicked. Such an operand is a temporary whose branches are consuming, as `ownership`'s "Consuming Positions" says ("Branch and elvis operands are consumed"). Not specific to boxes.
- **A `(&T)?` subject rejected every pattern of `T`** (`E4002`), so D4's `fun depth(t: (&Tree)?)` could read the tree only through `?.`. Patterns of `T` now match the referent with the borrow's mode, the same `Pattern::Deref` as a box ("A nullable borrow subject").

The safe-navigation lending gap of "Scenario fixes at implementation" is not a bug: the `references` rationale "A borrow points at one place or one temporary" makes a `?.` chain a consumed temporary on purpose and names lending it a possible relaxation, now a `ROADMAP.md` follow-up. Destructuring declarations through a box (`val (a, b) = h`, `E4024`) are a follow-up too.

## Risks / Trade-offs

- [Deep structures overflow the stack on drop, clone or comparison] → a "stack overflow" runtime error, not a crash (D14), and iterative drop is a recorded follow-up. Milestone 2's apps hold their lists in `List<T>` (`collections`), not in linked boxes.
- [Implicit lending, copy-out and the box view make boxes read like references, so a reader may not see a pointer] → that is the intent (D4). Every write (`*h = v`) and every move of contents (`unbox()`, consuming receivers) is still visible at its call site, apart from consuming receivers, which are implicit in every receiver mode.
- [The pattern part depends on `payload-patterns` landing first] → D1. The roadmap order must change, and the orchestrator owns that.
- [Equality glue changes the code of `==` on recursive types from inline to calls] → only at box layers, and comparing through a box was a pointer load already.

## Migration Plan

No external users. `h.toAccount()` through an owned box is now accepted (D7). `Heap<U>` to `Heap<T>` covariance is gone (D8), and nothing in the repository relies on it. The `E6001` fixtures move from `Heap<T>` to function values (D17). Any top-level declaration named `Heap` becomes `E4010`, and the repository has none. Rollback is a revert.

## Open Questions

- The exact wording of the `E4017` help for a box's contents, and of the invariance help, can be tuned during implementation.
- Whether `#equals` glue should also be used for large non-recursive types to bound code size; it can change without changing behaviour.
