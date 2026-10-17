# Design: reference-access

## Context

See `proposal.md` for motivation. This change is step 3 of "the road to the borrow checker" in `ROADMAP.md`. The user agreed its scope in the `/opsx:explore` session of 2026-10-17 (`docs/architecture.md` §3: "Reading and Writing Through References", the scenarios "Explicit mutation at call sites" and "Implicit shared borrow at argument sites", and the design notes). `value-semantics` left two items for it: moving out of a borrow reuses `E4017`, and borrowing a nullable's contents needed a design. While this change was being proposed on 2026-10-17, the user decided four more questions (D2, D5, D8, D9).

Current state (`crates/prism-typechecker/src/checker.rs` unless noted):

- **Arguments are consuming and must match exactly.** `check_expr_at(…, Origin::Argument)` sets the consuming flag, so `greet(u.name)` for `fun greet(n: &String)` would be checked as a move (`E4017`) if it got that far. Today it stops earlier, with `E4004`, because `String` is not assignable to `&String`. `g(a)` for `a: &Int` and `g(x: Int)` is also `E4004` (the `type-system` scenario "Borrow passed where a value is expected").
- **The operand of `&` is always read in place.** `infer_unary_op` infers it without the consuming flag, so `&(u.nick ?: "x")` and `&u?.name` are accepted today, borrowing a value that nothing owns. `&mut` goes through `infer_place`, which treats any non-place as a writable temporary.
- **`&mut r` on a `val r: &mut T` is accepted** (the "may be reborrowed mutably" branch of `infer_unary_op`), producing `&mut &mut T`, which no parameter accepts.
- **Reference assignability is covariant** (`types.rs`, `is_assignable_to`): `&U` is assignable to `&T` whenever `U` is assignable to `T`, so `&Int` passes as `&(Int?)`. `&mut U` to `&mut T` needs mutual assignability, and `&mut T` to `&T` is allowed.
- **`*` is only binary.** At the start of a line it ends the previous expression (`current_precedence` in `crates/prism-parser/src/parser.rs`), which is the behaviour the newline scenario needs. `parse_assignment_or_expression` accepts a variable, a `.` field access or an index as an assignment target. `UnaryOp` has `Neg`, `Not`, `Borrow` and `BorrowMut`.
- **Borrowed nullables are opaque.** `member_type` rejects `n?.id` for `n: &(User?)` with `E4006`, because it looks for `Nullable` before peeling references. The elvis typing keeps `&(String?)` as the left side's type instead of unwrapping it. `== null` already works, because operators peel references.
- **Re-pointing already typechecks.** `var r: &Int = &a; r = &b` passes `check_assignment`. `r = 5` on a `val r: &mut Int` reports both `E4003` and `E4002`.
- **`TypeError::CannotMoveOut`** always carries a `borrow` help string (`&u.name`), which does not fit a source that is already a borrow.
- **No smart casts and no methods.** `if (n != null) greet(n)` does not narrow `n`, and `x.f()` only calls function-typed fields or the built-in `clone()`.

## Goals / Non-Goals

**Goals:**
- Every implicit operation on a reference (borrow, reborrow, copy-out, nullable view) is decided by the type checker at one coercion point and classified, so the typed tree (step 5) records it instead of re-deriving it.
- A reference always points at one existing place or one temporary, never at a converted copy, so the MIR and the borrow checker see exactly one borrowed location per borrow.
- Every new scenario has a `.prism` fixture, and `E4019` has one.

**Non-Goals:**
- Borrow conflicts, use after move, and how long a borrow of a temporary in a `val` initializer lives (`val r = &make()`). These are steps 7 to 9. The type checker accepts that initializer today, and this change keeps accepting it.
- Method receivers, auto-borrowed receivers and two-phase borrows (step 4, `extension-methods`). The receiver exception in "Explicit mutation at call sites" stays in `docs/architecture.md`.
- `*` in read position, `*` on `Heap<T>`, and copy-out inside `if`/`when`/`?:` joins (`if (c) r else 5`). See D3 and D5.
- Smart casts, and borrowing through safe navigation (`&u?.nick` as an optional place).

## Decisions

Decisions marked **(user)** were made by the user, in the exploration session or while this change was proposed on 2026-10-17. The others follow from them or from existing rules, and are open to review here.

### D1. Implicit shared borrow at argument sites (user)

Where a call's parameter has type `&T`, an argument of type `T` is borrowed implicitly. This covers positional and named arguments, calls of function-typed values (`g(s)` for `g: (&String) -> Unit`) and, as an extension, default parameter values: a default is evaluated as the call's argument, so `fun greet(n: &String = "guest")` borrows a temporary. A parameter of type `(&T)?` takes a `T?` argument the same way (D8). Struct fields and enum payloads cannot hold borrows, so constructor arguments never borrow implicitly.

`&mut T` parameters are never borrowed implicitly. An owned place passed to one is `E4004`, with help suggesting `&mut x`. Mutation stays visible at the call site.

Outside arguments the borrow is written: `val r: &String = name` is `E4002` and `fun f(u: &User): &String = u.name` is `E4005`, each with help suggesting `&name` / `&u.name`.

Alternatives (from the exploration): explicit `&` everywhere, rejected as noise on the most common case once `String` is move-only. Swift/Hylo/Mojo-style parameter conventions, where `&T` is not a type, rejected because they would reshape the elision rules that the invariants commit to.

### D2. Places are borrowed in place; anything else is a temporary (user)

A **place** is a variable (local, parameter or top-level `val`), a `.` field path from any root (`u.name`, `t.lead.name`, `make().name`), or an index. Everything else is a temporary: literals, calls, constructors, safe navigations, `if`, `when`, blocks and `?:`. The same rule applies to an implicit argument borrow and to the operand of an explicit `&` or `&mut`:

- A place is read in place and not consumed. If its root is itself a temporary (`make().name`), that temporary lives until the end of the statement.
- A temporary is evaluated in a consuming position and then borrowed. It lives until the end of the statement. This is the consuming-position rule from `value-semantics`, so `greet(if (c) u.name else v.name)` and `&(u.nick ?: "x")` report `E4017` for the fields they would move into the temporary.

The user rejected distributing a borrow through branches, block tails and `?:` to their leaves. With distribution, `greet(if (c) u.name else v.name)` would borrow whichever field is chosen. The chosen rule matches Rust and gives one mental model, "a borrow points at one place or one temporary", and the MIR lowers each borrow to one location. The cost is that a right-hand fallback must be borrowed explicitly: `greet(&u.nick ?: &"anon")`, not `greet(u.nick ?: "anon")`.

Consequence for existing code: `&(…)` around a non-place and `&u?.name` are now checked as moves. Both are accepted today, and neither appears in a fixture or example.

Because a non-place argument may be a temporary or may already be a reference (`greet(longest(&a, &b))`), an argument whose parameter is a reference type is handled in two steps. A place is inferred without the consuming flag and then coerced (D10). Any other expression is inferred with the consuming flag. If its type is already assignable, it is passed as is; if its type is assignable to the referent, the temporary is borrowed; otherwise it is coerced or reported. Inferring rather than checking such an argument loses the expected type inside its branches. That is harmless, because a closure or `null` never needs `&T` as its expected type.

### D3. Implicit copy-out of `Copy` values (user)

Where a `T` is expected and the expression's type is a reference, the checker peels reference layers (`&`, `&mut`, nested: `&&Int`) until the type is assignable to `T`. If the result is `Copy`, the value is copied out. Otherwise it is a move out of a borrow (D4). The value positions are initializers with an annotation, assigned values (including `*r = v`), returned values, by-value arguments, constructor fields and payloads, and default values. Operators and conditions already read through references (`typechecker-soundness`, D1) and are unchanged.

Copy-out also applies through one `?`: `(&T)?` is accepted where `T?` is expected when `T` is `Copy`, because `T?` is then `Copy` too and the nullable view (D8) makes `(&T)?` common.

An unannotated `val b = a` for `a: &Int` keeps the type `&Int`, a copy of the borrow, since `&T` is `Copy`. Nothing is expected there, so nothing is copied out.

Copy-out in joins is not part of this change: `if (c) r else 5` and `first() ?: 0` with a `&Int` side still need both sides to have one type. Applying copy-out there would make a join's type depend on which side is a reference. It is listed as a possible relaxation.

### D4. Moving out of a borrow reuses `E4017` (user, from `value-semantics`)

When D3's peeled type is assignable but not `Copy`, the checker reports `CannotMoveOut` (`E4017`) with the source "borrow `u`", where `u` is the expression's text (`borrow 'get()'` for a call). The help suggests `u.clone()` when the type is `Clone`. Otherwise it suggests taking a `&T` where the value is expected. It never suggests `&u`, because `u` is already a borrow. In the error type, `borrow` becomes optional, and the CLI renders whichever help lines are present. Rust uses one code, E0507, for every place behind a reference, and the fixes are the same as for a field: clone it, or borrow it instead.

### D5. Prefix `*` writes through a reference, and only there (user)

Grammar: an assignment target may start with one or more `*`. The operand after the last `*` is parsed at prefix precedence, so postfix operators bind tighter: `*r.x = 1` is `*(r.x) = 1`, and `*pick(&mut a, &mut b) = 0` is accepted. The parser builds `Unary { op: Deref, … }`, nested once per `*`, and only at the top of an assignment target. This is handled in `parse_assignment_or_expression`, so statements and unbraced branch and loop bodies all accept it. A `*` that starts any other expression is `E2001` with the message "prefix `*` writes through a reference and is only allowed on the left of `=` (`*r = v`); reads through references are implicit". `starts_expr` accepts `*` so that `return *r` reaches that message instead of a generic one. A newline before `*` still ends the previous expression, by the existing rule.

Typing in `check_assignment`: the operand is read in place, never consumed, and must have a reference type. Every reference layer that is written through must be `&mut`; otherwise the result is `E4003`, "cannot write through a shared borrow", following `contents_writable`. A non-reference operand (`*x = 1` for `x: Int`) is a new kind of error, `NotAReference` (`E4019`). The value is checked against the referent in a consuming position, so D3 and D4 apply (`*r = s` for `s: &Int` copies out). The binding that holds the reference may be a `val`, because writing the referent does not change the binding.

At runtime the write drops the old referent and moves the value in (recorded in `docs/architecture.md` §4).

Rejected: `*` as a general place operator that also reads (`val x = *r`, `&mut *r`). The user chose writes only: reads have one spelling, the implicit one, as the exploration's design note says. The cost is that no syntax reborrows a `&mut` into a local (`val r2 = &mut *r`), which is listed as a follow-up. Also rejected: a separate statement kind with a dereference count. `Deref` as an operator is the same projection the typed tree and MIR places will use, and it nests naturally (`**rr`). A `Deref` outside a target can only come from a hand-built AST. The checker treats it as an internal error (a debug assertion, then the error type).

### D6. Assigning a `var` of reference type re-points it (user)

`r = v` assigns the reference itself, as it already does. Nothing changes for valid code. Only the diagnostics change: when `r` has a reference type and `v`'s type fits the referent (after D3) but not the reference type, the error carries help. For a `&mut` reference the help is "to write through the reference, use `*r = v`". For a `&` reference it says that a shared borrow cannot be written through. A `val` binding reports only `E4003` for this, not also `E4002`. A `var` binding reports `E4002` with the help.

Rejected (exploration): C++-style transparent references, which cannot be re-pointed, and a type-directed `r = v` that writes through or re-points depending on `v`'s type, which is hard to read.

### D7. Passing a `&mut` place reborrows it (user)

A place of type `&mut T` passed where `&mut T` or `&T` is expected is reborrowed, not moved, so `bump(r); bump(r)` keeps `r` usable. Typing does not change, because such an argument is already assignable. The coercion step classifies it as a reborrow, for the typed tree and the move checker. A temporary of type `&mut T` (`bump(pick(&mut a, &mut b))`) is passed as is. A `&mut T` in any other position (`val r2 = r`) is moved, as `value-semantics` already allows.

`&mut r` on a `val r: &mut T` now follows the ordinary rule and reports `E4003`. `&mut r` borrows the binding, not its referent. With write-through, `*(&mut r) = &mut other` would re-point a `val`. Implicit reborrowing makes the exception unnecessary.

### D8. Borrowed nullables: an implicit view `&(T?)` to `(&T)?` (user)

- At value positions, `&(T?)` converts to `(&T)?`, and `&mut (T?)` converts to `(&mut T)?` or to `(&T)?`. The conversion reads the tag and gives `null` or a pointer to the payload. It never allocates, but it is a conversion, not subtyping (D9), so it lives in the coercion step. The opposite direction does not exist, because a `(&T)?` has no storage for a `T?` to point at.
- `?.` on an expression whose type is references around a nullable peels the references first, so `n?.id` for `n: &(User?)` reads `id` in place.
- `?:` with a left side of type `&(T?)` (or `&mut (T?)`) treats it as `(&T)?` (`(&mut T)?`), so `n ?: &"anon"` has type `&String`.
- At an argument site, a place of type `T?` passed to a `(&T)?` parameter is borrowed where it lives (D1, D2): `show(u.nick)`.

This is the idiom `value-semantics` left open: `greet(&u.nick ?: &"anon")` and `show(u.nick)` work without cloning, and `u.nick?.clone() ?: "anon"` remains the way to get an owned `String`.

Rejected: an explicit built-in member (`u.nick.asRef()`, Rust's `Option::as_ref`), which is noise on a common Kotlin idiom, and deferring the question, which would leave cloning as the only way to read a borrowed nullable.

### D9. References are invariant (user)

`&U` is assignable to `&T` only when `U` and `T` are the same type, except that a shared view may weaken mutability: `&(&mut V)` is assignable to `&(&V)`. `&mut U` is assignable to `&mut T` only when `U == T`. `&mut U` is assignable to `&T` when `&U` is. `Nothing` and the error type keep their top-level rules. Nullables of references follow the existing nullable rules: `(&mut T)?` is assignable to `(&T)?`, and `&T` to `(&T)?`.

A reference points at existing storage. `T` to `T?` changes the value's representation (a tag is added), which a value conversion can do but a pointer cannot. Keeping covariance would make codegen materialize a converted temporary behind the "borrow", a hidden copy that is not the original, and a hidden clone for non-`Copy` types. `Heap<U>` to `Heap<T>` has the same problem; it is left to the `Heap<T>` change and listed as a follow-up.

### D10. One coercion step, returning an adjustment

A single function, `coerce(found, expected, site, is_place) -> Result<Adjustment, …>`, runs at every value position after plain assignability fails, and at argument sites whose parameter is a reference (D2). It tries, in order: implicit borrow (`Borrow` of a place, `BorrowTemporary`, or `BorrowContents` for `T?` to `(&T)?`, argument sites only); `Reborrow` (D7); the nullable `View` (D8); and `CopyOut { layers, through_nullable }` (D3). Then it falls back to `E4017` (D4) or the origin's mismatch code with D1/D6 help. `Adjustment::None` covers the plain assignable case. Because `check_expr_kind` already pushes the expected type into `if`/`when` branches in check mode, copy-out applies at each branch that needs it.

For a place, the leaf move check (`check_move_source`) now runs after the coercion, and only when the value is actually moved (`Adjustment::None` on a non-`Copy` type). A borrow or copy-out never reports a move, and a value that fails to coerce reports only the mismatch: `fun g(u: &User): &String = u.name` gets `E4005` with help suggesting `&u.name`, not also `E4017`. Pass-through forms keep the value-semantics flow, in which their leaves are checked as they are inferred.

The checker does not store adjustments yet: there is no typed tree. Returning them now keeps the rules in one place, and gives step 5 a value to record rather than logic to re-derive. Rejected: folding the conversions into `is_assignable_to`, which is subtyping and is also used for joins and equality, where silent borrows and copies would be wrong.

### D11. A new `references` capability spec

These rules are about references rather than owned values, and `borrowck-exclusivity` and `borrowck-elision` will add borrow conflicts and lifetimes about the same subject. Rust's book splits ownership from references and borrowing for the same reason. `ownership` keeps the consuming-position and move rules that this change extends, and `type-system` keeps assignability and operators, modified to point at `references`. Rejected: putting everything in `ownership` (which would mix value and reference rules) or in `type-system` (already the largest spec).

### D12. Error codes

- `E4017` `CannotMoveOut`: gains the source "borrow `u`" (D4).
- `E4019` `NotAReference`: new, the next free code. `*` applied to a value that is not a reference (D5).
- `E4003` `ImmutabilityViolation`: also covers writing through a shared borrow (D5), and `&mut r` on a `val` reference binding (D7).
- `E4002`, `E4004`, `E4005`: unchanged meaning, with new help lines (D1, D6).

### D13. Existing tests

- Two expectations flip, and the modified specs record both. The `prism-typechecker` regression test `borrow_passed_where_a_value_is_expected` (`g(a)` for `a: &Int`, `E4004` today) now expects acceptance, as `references`' "Copy-out of a borrowed primitive". It gains a `String` counterpart that expects `E4017`, the modified `type-system` scenario. The fixture `type-system/reference-and-borrow-compatibility--plain-value-for-borrow-parameter-rejected` (`read(x)` for `fun read(r: &Int)`) becomes `--plain-value-borrowed-implicitly`, expecting `ok`. A new `--plain-value-for-mutable-parameter-rejected` keeps `E4004` covered.
- Fixtures and `prism-typechecker` tests that rely on covariant references or on `&mut r` for a `val` reference binding are rewritten to keep their scenario. None is known: the regression test that `&mut Int` must not become `&mut Int?` already expects rejection.

## Risks / Trade-offs

- [`greet(u.nick ?: "anon")` is rejected (`E4017` for `u.nick`), although it looks like the borrowed form] → `E4017`'s help for a field already suggests borrowing (`&u.nick`) and cloning (`u.nick?.clone()`). The `references` spec's scenarios and the `E4017` catalog entry show the borrowed elvis form, `&u.nick ?: &"anon"`. Distributing borrows (rejected in D2) would remain possible later, and would only accept more programs.
- [Copy-out hides a dereference] → only `Copy` values are copied, and the copy is bitwise and never allocates, as Kotlin readers expect from `val x: Int = a`. Non-`Copy` values still need an explicit `clone()`.
- [An implicitly borrowed temporary lives only until the end of the statement] → this is enough for arguments, whose borrows end when the call returns unless elision ties the result to them. That case is the borrow checker's (step 9), which then reports a dangling borrow, as Rust does.
- [Invariance and the new `&` operand rule reject code accepted today (`&u?.name`, `&Int` as `&(Int?)`)] → neither appears in fixtures or examples. Both were unsound for codegen, and the help suggests `?.clone()` or an explicit conversion.
- [Writes-only `*` leaves no syntax for reborrowing a `&mut` into a local] → pass it as an argument (D7), or move it. A reborrow syntax is listed as a follow-up.
- [Arguments to reference parameters are inferred rather than checked (D2)] → expected types matter for closures and `null`, and neither is ever borrowed.

## Migration Plan

No external users. In-repo: the fixture in D13 is replaced in the same commit as the coercion step, and any test relying on covariance is rewritten in the commit that makes references invariant. Rollback is a revert.

## Open Questions

- The exact wording of the new help lines can be tuned during implementation, provided `E4017` keeps the "cannot move out of {source}" shape.
