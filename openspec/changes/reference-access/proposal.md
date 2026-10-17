# Proposal: reference-access

## Why

`value-semantics` made moves explicit, but a reference still can't be used like the value it refers to. `g(a)` for `a: &Int` is rejected, a `String` can only be passed to a `&String` parameter as `&name`, there is no way to write a whole `Int` through a `&mut Int`, and a borrowed nullable (`&(String?)`) can't be unwrapped without cloning. This change, step 3 of "the road to the borrow checker" in `ROADMAP.md`, sets how references are created at call sites, read, written, re-pointed and passed on. Steps 5 to 9 can then lower every borrow, copy-out and write-through to the MIR as an explicit operation. The rules are flow-insensitive, so they belong to the type checker (`E4xxx`).

## What Changes

- **Implicit shared borrow at argument sites.** Where a parameter has type `&T`, an argument of type `T` is borrowed implicitly: a place (`greet(name)`, `greet(u.name)`) is borrowed where it lives, and any other expression (`greet("Ada")`, `greet(make())`, `greet(if (c) a else b)`) is evaluated into a temporary that lives until the end of the statement. Anywhere other than an argument, the borrow is still written (`val r: &User = &u`), and `&mut` is always written.
- **Implicit copy-out.** Where a `T` is expected and a `&T` or `&mut T` is given, a `Copy` value is copied out (`g(a)`, `val x: Int = a`, `return p` for `p: &Point`). A non-`Copy` value is rejected as a move out of a borrow, reusing `E4017` with the source "borrow `u`", and its help suggests `u.clone()` when the type is `Clone`.
- **Prefix `*` writes through a reference**, and only there: `*r = r + 1`, `**rr = 0`. `*` anywhere else is a syntax error whose message says that reads through references are implicit. Writing through a `&` is `E4003`, and applying `*` to a value that is not a reference is a new code, `E4019` (`NotAReference`).
- **Assigning to a `var` of reference type re-points it** (`r = &b`). Assigning a value of the referent's type (`r = 5`) is rejected, with help suggesting `*r = 5`.
- **Passing a `&mut` binding reborrows it** without writing `&mut`, so the binding stays usable after the call. **BREAKING:** `&mut r` on a `val` binding of type `&mut T` now follows the ordinary `val` rule (`E4003`). It used to be accepted, producing a `&mut &mut T` that no parameter accepted.
- **Borrowed nullables.** `&(T?)` converts implicitly to `(&T)?`, and `&mut (T?)` to `(&mut T)?` or `(&T)?`. `?.`, `?:` and `== null` read through a reference to a nullable, and a `T?` place passed to a `(&T)?` parameter is borrowed where it lives. `greet(&u.nick ?: &"anon")` and `show(u.nick)` work without cloning.
- **BREAKING: references are invariant.** `&U` is assignable to `&T` only when `U` and `T` are the same type. The exceptions are `&mut` weakening to `&` at the outer layer or behind shared borrows, and `Nothing`. Previously `&Int` was accepted as `&(Int?)`, although the two have different layouts.
- **BREAKING: a borrow of a non-place is a borrow of a temporary.** `&(if (c) u.name else v.name)` and `&(u.nick ?: "x")` now report `E4017`, because the branch or elvis result is a new value made by moving the field. Borrows do not pass through branches to their leaves (user decision).
- Not in scope: use after move, borrow conflicts and lifetimes (steps 7 to 9); method receivers and two-phase borrows (step 4); `*` for reads or for `Heap<T>`; copy-out inside `if`/`?:` joins (`if (c) r else 5`); smart casts; borrowing through safe navigation (`&u?.nick`).

## Capabilities

### New Capabilities
- `references`: how references are created at call sites, read (copy-out), written (`*r = v`), re-pointed, reborrowed and viewed through nullables, and how reference types relate. `borrowck-exclusivity` and `borrowck-elision` later add borrow conflicts and returned-borrow lifetimes here.

### Modified Capabilities
- `ownership`: "Consuming Positions" says when the operand of `&` is read in place and when it is a temporary, and "Moves Take Whole Values Only" adds the borrow source to `E4017`.
- `type-system`: "Operators Read Through References" no longer forbids `&T` in value positions, and "Static Type Inference and Assignment Compatibility" makes references invariant and allows implicit shared borrows at arguments.
- `syntax`: assignment targets may be written through prefix `*`, and `*` is rejected everywhere else.

## Impact

- `crates/prism-ast`: `UnaryOp::Deref` (`*`), produced by the parser only at the top of an assignment target.
- `crates/prism-parser`: parses `*`… targets in `parse_assignment_or_expression` (statements, branch and loop bodies), and gives `*` in expression position a targeted `E2001` message.
- `crates/prism-typechecker`: one coercion step at value positions (implicit borrow, reborrow, nullable view, copy-out) that classifies each adjustment for the later typed tree. Also place detection for `&` operands and arguments, write-through assignments, re-point diagnostics, invariant reference assignability, `?.`/`?:` through borrowed nullables, and the `&mut` rule for `val` bindings.
- `crates/prism-cli`: renders `E4019` and the new `E4017`, `E4002`, `E4003`, `E4004` and `E4005` help lines. New fixtures go under `tests/fixtures/references/`. Two existing expectations flip to acceptance: the regression test for `g(a)` with `a: &Int`, and the fixture passing a plain `Int` to a `&Int` parameter (design D13).
- `openspec/specs/diagnostics/spec.md`: `E4019` is added, and the `E4003` and `E4017` rows are widened. The catalog is not a requirement, so it is edited directly.
- `docs/architecture.md`: §4 records the runtime meaning of write-through, argument temporaries and the nullable view. On archive, "Reading and Writing Through References", the implemented parts of "Explicit mutation at call sites" and "Implicit shared borrow at argument sites", and the matching design notes move into the specs. The receiver exception stays for `extension-methods`.
