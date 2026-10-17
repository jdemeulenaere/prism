# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/`, one case per scenario, or per form of a scenario with several (`<scenario-slug>--<form>`), as the header of `fixture_tests.rs` describes.

## 1. Write-through grammar (D5)

- [x] 1.1 In `prism-ast`, add `UnaryOp::Deref` (`as_str` `"*"`), documented as produced only at the top of an assignment target. In `prism-parser`, have `parse_assignment_or_expression` parse one or more leading `*` followed by an operand at `Precedence::Prefix`, build one nested `Deref` per `*`, and require `=`. Accept the result as a place in the target check. Add a `Star` arm to `parse_prefix` reporting the D5 `E2001` message at the `*`, and add `Star` to `starts_expr`. Verify with parser tests and fixtures under `syntax/` for every scenario of "Writing Through a Reference" and the modified "Invalid assignment target rejected", and that `cargo test -p prism-parser` passes.

## 2. Invariant references (D9)

- [x] 2.1 In `prism-typechecker/src/types.rs`, make `is_assignable_to` invariant for references per D9: `&U` to `&T` when `U` and `T` are the same up to `&mut` becoming `&` behind shared borrows; `&mut U` to `&mut T` only when `U == T`; `&mut U` to `&T` whenever `&U` is. Keep the `Nothing`, error and nullable rules. Verify with unit tests for each rule, fixtures under `references/` for "A borrowed value is not a borrowed nullable" and "Mutable borrows weaken to shared ones", and that the `type-system` fixtures for "Borrowed constructor values match annotated references" and the regression test that `&mut Int` must not become `&mut Int?` still pass. Rewrite any test that relied on covariance so it keeps its scenario (D13).

## 3. Coercion step, copy-out and moves out of borrows (D3, D4, D10)

- [x] 3.1 In `prism-typechecker`, add an `Adjustment` enum and a `coerce` function per D10, with `CopyOut { layers, through_nullable }` and the `E4017` fallback for this group. The borrow, reborrow and view variants come in groups 4 and 5. Call it from `check_expr_kind`'s default path when assignability fails. For a place, infer it without the consuming flag and run `check_move_source` only when the coerced value is moved, so a failed coercion reports only the mismatch. Make `CannotMoveOut`'s `borrow` help optional and add the "borrow `u`" source, with D4's help. Verify with fixtures under `references/` for every scenario of "Copy-Out From References", under `ownership/` for every scenario of "Moving Out of a Borrow Is Rejected" (one case per site in "Moves out of a borrow at every consuming site"), and under `type-system/` for "Borrow passed where a value is expected", and that the existing `ownership/` fixtures still pass.
- [x] 3.2 Flip the regression test `borrow_passed_where_a_value_is_expected` in `crates/prism-typechecker/tests/regressions.rs` to expect acceptance, and add its `String` counterpart expecting `E4017` (D13). Render the new `E4017` help in `prism-cli`, with a CLI rendering test for one `Clone` and one non-`Clone` borrow source. In `openspec/specs/diagnostics/spec.md`, widen the `E4017` row to cover a borrow given where a value is expected. Verify that `cargo test --workspace` passes.

## 4. Implicit borrows, places and reborrows (D1, D2, D7)

- [x] 4.1 Add a place predicate (variable, `.` field path from any root, index) per D2. At argument sites whose parameter type is a reference, including default values and calls of function-typed values, infer a place without the consuming flag and coerce it (`Borrow`, `Reborrow`). Infer any other argument with the consuming flag and borrow it as a temporary when its type fits the referent (`BorrowTemporary`). Never borrow `&mut` implicitly. Verify with fixtures under `references/` for every scenario of "Implicit Shared Borrow at Argument Sites", "Explicit Mutable Borrows at Call Sites" and "Passing a Mutable Borrow Reborrows It" (the val-binding scenario after 4.2), and with the `type-system` fixture renames in D13 (`--plain-value-borrowed-implicitly` expecting `ok`, a new `--plain-value-for-mutable-parameter-rejected` expecting `E4004`).
- [x] 4.2 In `infer_unary_op`, infer the operand of `&` and `&mut` with the consuming flag when it is not a place (D2), and remove the exception that lets `&mut` borrow a `val` binding of type `&mut T` (D7). Add the D1 help lines: `&mut x` on `E4004`, and `&x` / `&u.name` on `E4002` and `E4005` when the value would fit the expected reference if borrowed. Render them in `prism-cli`, and update the `E4003` catalog row for `&mut` of a `val` reference binding. Verify with fixtures under `ownership/` for "Borrowing a branch or elvis result" and "Borrowing a safe navigation", under `references/` for "Mutable borrow of a val reference binding rejected" and "Borrows outside arguments are written", and that the existing `reference-and-borrow-compatibility--*` fixtures still pass.
- [x] 4.3 In `docs/architecture.md` §4, add the planned runtime meaning of implicit argument borrows: a borrowed place is not copied, and an argument temporary is dropped at the end of its statement. Verify the section reads as planned, not implemented.

## 5. Borrowed nullables (D8)

- [x] 5.1 Add the `View` adjustment (`&(T?)` to `(&T)?`, `&mut (T?)` to `(&mut T)?` and `(&T)?`) and, at argument sites, `BorrowContents` for a `T?` place passed to `(&T)?`. In `member_type`, peel references before looking for the nullable under `?.`. In the elvis typing, treat a left side of type `&(T?)` / `&mut (T?)` as `(&T)?` / `(&mut T)?`. Verify with fixtures under `references/` for every scenario of "Borrowed Nullables Expose Their Contents".
- [x] 5.2 In `docs/architecture.md` §4, add the planned runtime meaning of the view: it reads the tag and gives `null` or a pointer to the payload, without allocating. Verify the section reads as planned, not implemented.

## 6. Writing through references and re-pointing (D5, D6)

- [x] 6.1 In `check_assignment`, type a `Deref` target per D5: read the operand in place, require a reference type (else the new `TypeError::NotAReference`, `E4019`), require `&mut` at every layer written through (else `E4003`, "cannot write through a shared borrow"), and check the value against the referent in a consuming position. Treat a `Deref` outside a target as an internal error. Render `E4019` in `prism-cli`, add it to the diagnostics catalog, and widen the `E4003` row to writes through a shared borrow. Verify with fixtures under `references/` for every scenario of "Writing Through a Mutable Reference", and that the fixture harness's catalog check passes.
- [x] 6.2 Add the D6 diagnostics for `r = v` where `r` has a reference type and `v` fits the referent: a `val` reports only `E4003`, a `var` reports `E4002`, each with help suggesting `*r = v` for `&mut` (or saying a shared borrow cannot be written through). Verify with fixtures under `references/` for every scenario of "Re-Pointing Reference Variables", and for the `r = 5` part of "Writing through a mutable borrow".
- [x] 6.3 In `docs/architecture.md` §4, add the planned runtime meaning of `*r = v`: the old referent is dropped and `v` is moved in. Verify the section reads as planned, not implemented.

## 7. Integration

- [x] 7.1 Update `examples/` where a call now reads better with an implicit borrow, and confirm the explicit forms still check. Run the apply guidance in `openspec/config.yaml`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `prism check` on every file under `examples/`, and `openspec validate --all --strict`. Verify all pass.

## Workflow follow-up

- On archive, create `openspec/specs/references` from the delta, and promote D1–D10 into its `## Design Rationale`, D2 and D4 into `ownership`'s, D9 into `type-system`'s, and D5 into `syntax`'s. Update `type-system`'s D1 rationale ("Operators read through references; value positions do not"), which this change supersedes.
- In `docs/architecture.md` §3, remove the "Reading and Writing Through References" requirement and the "Implicit shared borrow at argument sites" scenario. Reduce "Explicit mutation at call sites" to the method-receiver exception, for `extension-methods`. Remove the design notes on implicit shared borrows and prefix `*` (moved to the specs), and update the "Implemented today" paragraph.
- Tick step 3 of "the road to the borrow checker" in `ROADMAP.md`, and add these follow-ups as unchecked items, each naming `reference-access`:
  - a syntax for reborrowing a `&mut` into a local (`val r2 = …`), since `*` is writes-only;
  - possible relaxation: copy-out in `if`/`when`/`?:` joins (`if (c) r else 5`, `first() ?: 0`);
  - possible relaxation: distributing borrows through branches and `?:` (`greet(u.nick ?: "anon")`), rejected for now;
  - smart casts (`if (n != null) greet(n)`), and borrowing through safe navigation (`&u?.nick`) as an optional place;
  - how long an explicitly borrowed temporary in a `val` initializer lives (`val r = &make()`), for `borrowck-exclusivity` or `borrowck-elision`;
  - for the `Heap<T>` item: `Heap<U>` to `Heap<T>` covariance changes representation like references did, and whether `*h = v` writes a `Heap`'s contents.
