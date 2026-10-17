# Tasks

## 1. Top-level `var` removed (D9)

- [x] 1.1 In `prism-parser`, reject `var` at top level with the D9 `E2001` message pointing at `var`, for every visibility. Remove `TopLevelVarDecl.is_var` from `prism-ast`, `ExportedSymbol::Var.is_var` from `prism-abi` (and its hash byte), and the `var`/`val` rendering of top-level values in `prism-cli`'s ABI dump, and simplify the type checker's top-level registration to immutable bindings. Verify with fixtures `ownership/top-level-var-rejected--{private,public,package}` and `ownership/top-level-val-and-local-var-accepted`, and that `cargo test --workspace` passes.

## 2. Marker grammar (D2)

- [x] 2.1 In `prism-ast`, add `enum TypeMarker { Copy, Clone }` and `marker: Option<TypeMarker>` on `StructDecl` and `EnumDecl`. In `prism-parser`, parse the optional `: Copy` or `: Clone` after the type parameters (block structs, enums) and after the shorthand parameter list, rejecting other names, two names, an empty `:` and a misplaced marker with the D2 `E2001` message. Update hand-built ASTs in unit tests. Verify with parser tests and fixtures for the `syntax` scenarios "Markers on a block struct and an enum", "Marker after type parameters and a shorthand parameter list", "Declaration without a marker", "Other names after the colon rejected", "Marker in the wrong place rejected" and "Marker names remain identifiers".

## 3. Markers in the ABI (D6, D7)

- [x] 3.1 In `prism-abi`, add the marker to `ExportedSymbol::Struct` and `Enum` and to `AbiType::Nominal`. Have Phase 1a's own-type and exported-type maps carry `(kind, marker)` and the `NameBinder` return it, and hash the marker as one byte wherever it appears. Update `prism-cli`'s workspace binder and ABI dump to pass and print it. Verify with Rust ABI scenario tests for "Adding or changing a marker changes the ABI hash", "Marker on a private type", "Referenced type records its marker" and "Changing a referenced type's marker".
- [x] 3.2 Add `AbiType::Range` with its own hash tag, recorded when `Range` binds to no own or imported type, and lower it in `Type::from_abi` to the built-in `Range`. Verify with an ABI scenario test that `public fun f(r: Range): Range` records `AbiType::Range` while a user `public struct Range` still records a nominal, and (after 4.1) with the fixture `compilation-and-abi/built-in-range-in-an-exported-signature`.

## 4. The predicates and the declaration check (D3, D4)

- [x] 4.1 In `prism-typechecker`, add one `NominalId -> Option<TypeMarker>` map filled from local declarations, imported structs and enums, and every ABI nominal reference ingested (D3), and `Checker::is_copy` and `Checker::is_clone` implementing the D3 table. Verify with unit tests covering every row of the table, including a nominal type known only through a reference.
- [x] 4.2 After registration, check every marked struct and enum, reporting `InvalidMarkerDeclaration` (`E4016`) once per field or payload that is not `Copy` (for `: Copy`) or not `Clone` (for `: Clone`), naming the marker, the field (or variant and payload position) and its type. Render it in `prism-cli` with a help line, and add `E4016` to the catalog in `openspec/specs/diagnostics/spec.md`. Verify with fixtures under `ownership/` for every scenario of "Built-In Copy Types", "Built-In Clone Types" and "Marker Declarations Are Checked Structurally" (one case per form in the "Non-Copy built-ins…" and "Non-Clone built-ins…" scenarios), and that the fixture harness's catalog check passes.

## 5. Explicit `clone()` (D5)

- [x] 5.1 In `infer_call_expr`, type a call whose callee is a member access or safe navigation named `clone` as the built-in member when the receiver's type has no field named `clone` (D5): peel `&`/`&mut` from the receiver type, give `T` (or `T?` through `?.`) when it is `Clone`, report `E4004` for arguments, and otherwise report `NotCloneable` (`E4018`) with the D5 help. Render `E4018` in `prism-cli` and add it to the diagnostics catalog. Verify with fixtures under `ownership/` for every scenario of "Explicit Clone" and for "Clone struct is not Copy" and "Copy implies Clone".
- [x] 5.2 In `docs/architecture.md` §4 ("Runtime Semantics Awaiting Code Generation"), add the planned meaning of `clone()` as a requirement with scenarios: a `Copy` value is copied bitwise; a `String` clone allocates a new buffer with the same contents; a `Heap<T>` clone allocates a new box holding a clone of `T`; `T?` clones its contents when present; a user struct or enum clones each field or payload, in declaration order. Verify the section reads as planned, not implemented.

## 6. Move sources (D8, D11)

- [x] 6.1 Add `check_move_source` and call it at every consuming position listed in D8 (both `check_expr_at` and the `infer_expr` paths for unannotated initializers), passing through `if`/`when` branches, block tails, parentheses and `?:` operands. Report `CannotMoveOut` (`E4017`) with the D8 message for non-`Copy` fields, safe navigations, indexes and top-level `val`s, never for the error type, with help suggesting a borrow and, for `Clone` types, `.clone()` (`?.clone()` for nullable sources). Render it in `prism-cli` and add `E4017` to the diagnostics catalog. Verify with fixtures under `ownership/` for every scenario of "Copy and Clone Are Declared on Structs and Enums", "Consuming Positions" and "Moves Take Whole Values Only" (one case per site in "Field moves at every consuming site" and per form in "Moving a field through a borrow or Heap" and "Reads in place are not moves"), with the help text checked by a CLI rendering test for one `Clone` and one non-`Clone` source.
- [x] 6.2 Rewrite the pre-existing fixtures and `prism-typechecker` regression tests that 6.1 now rejects, so they still exercise their scenario without a move (D11): `u.name.clone()`, `u?.name?.clone()`, or `&u.name` and comparisons where a clone would obscure the scenario. Known cases are `field-access-through-a-borrowed-reference--*`, `safe-navigation-and-elvis-operator-evaluation--{elvis-after-safe-navigation,safe-navigation-gives-nullable-field-type}` and the `regressions.rs` tests reading `u.name`, `o.user.name` or `b?.a?.name`; run the suite to find the rest. Verify that `cargo test --workspace` passes with no fixture's expectation changed.

## 7. Integration

- [x] 7.1 Run the apply guidance in `openspec/config.yaml`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `prism check` on every file under `examples/`, and `openspec validate --all --strict`. Verify all pass.

## Workflow follow-up

- On archive, promote D1–D10 into a `## Design Rationale` section of the new `ownership` spec, D2 and D9 into `syntax`'s, and D6–D7 into `compilation-and-abi`'s.
- In `docs/architecture.md` §3, remove the "Copy Types" requirement, the "Top-level var rejected" scenario, the "Moves take whole values only" scenario's field, `Heap<T>` and top-level parts (keep "out of a borrow" for `reference-access`), and the design notes on opt-in `Copy`, top-level `var` and `String`, and update the "Implemented today" paragraph. The use-after-move scenarios "Plain enum is move-only without the marker" and "String is move-only" stay for `borrowck-moves`, reworded so they no longer restate the `Copy` table.
- Tick step 2 of "the road to the borrow checker" in `ROADMAP.md`, and add the follow-ups as unchecked items, each naming `value-semantics`:
  - rejecting borrows in enum payloads;
  - `Copy` and `Clone` bounds on type parameters (append to the generics item);
  - custom `clone()` bodies once traits exist (append to the traits item);
  - moving a field out of a temporary, as a possible relaxation next to partial moves;
  - for `reference-access`: extending `E4017` with the "borrow" source, and borrowing a nullable's contents without cloning (`&(T?)` to `(&T)?`);
  - for `extension-methods`: rejecting an extension named `clone` on a `Clone` type.
