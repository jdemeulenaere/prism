# Proposal: value-semantics

## Why

The borrow checker planned in `ROADMAP.md` ("the road to the borrow checker") has to know which values move and which are copied. Today the type checker has no notion of `Copy`: a `String` field, a `Heap<T>` and an `Int` are all just values. This change, step 2 of that road, makes the ownership model concrete in the type system. Which types are `Copy` or `Clone` is declared and recorded in the ABI. A move must take a whole owned value. An explicit `.clone()` is the way to get an owned copy of a non-`Copy` value. Top-level state is immutable. Step 7 (`borrowck-moves`) can then track use after move over locals alone. Every rule here is flow-insensitive, so it belongs to the type checker (`E4xxx`) rather than to the borrow checker (`E5xxx`), per `docs/architecture.md` §3.

Without `.clone()`, whole-value moves would leave no way to get an owned `String` out of a struct while keeping the struct (`fun f(u: &User): String = u.name`, `u.nick ?: "anon"`), so the user decided to design cloning now rather than defer it.

## What Changes

- **Opt-in `: Copy` and `: Clone` markers on struct and enum declarations**, in Kotlin supertype position: `struct Point : Copy { … }`, `struct User : Clone { … }`, `struct Id(val v: Int) : Copy`, `enum Color<T> : Copy { … }`. Exactly one of `Copy` and `Clone` may follow the colon, and `Copy` implies `Clone`. The type checker accepts a marker only when every field or variant payload is `Copy` (for `: Copy`) or `Clone` (for `: Clone`). Otherwise it rejects the declaration with a new code, `E4016` (`InvalidMarkerDeclaration`), naming the offending field.
- **The built-in tables.** `Copy`: `Int`, `Float`, `Bool`, `Char`, `Unit`, `Nothing`, `Range` and `&T`, and `T?` when `T` is. `Clone`: every `Copy` type and `String`, and `T?` and `Heap<T>` when `T` is. Neither: `&mut T`, function types, type parameters and unknown (opaque) type names.
- **Explicit `.clone()`.** Every `Clone` type has a built-in `clone()` member. It reads its receiver in place, looking through `&` and `&mut`, and returns an owned copy, field by field for user types: `u.name.clone()`, `u.nick?.clone() ?: "anon"`. Calling it on a type that is not `Clone` is a new code, `E4018` (`NotCloneable`), whose help suggests `: Clone`. The name is `clone`, not `copy`, because Kotlin's data-class `copy()` is a shallow copy with named overrides.
- **Markers are part of the ABI.** An exported struct or enum records its marker, and so does every reference to a nominal type in an exported signature. Adding, removing or changing a marker changes the ABI hash.
- **Moves take whole values only.** Wherever a value is consumed (an initializer, an assigned value, a by-value argument, a returned value, a constructor field or payload), a non-`Copy` value must be a whole owned local, a by-value parameter or a temporary. Moving a non-`Copy` value out of a field (including through a borrow, through `Heap<T>` or out of a temporary), out of a safe-navigation access, or out of a top-level `val` is rejected with a new code, `E4017` (`CannotMoveOut`). Its help suggests a borrow (`&user.name`) and, for a `Clone` type, a clone (`user.name.clone()`). Reading `Copy` fields, borrowing fields, cloning them, and using them as operator operands, conditions or `when` subjects are unaffected.
- **BREAKING: top-level `var` is removed.** A top-level declaration is `fun`, `struct`, `enum` or `val`, and the parser rejects `var` at top level with `E2001`. Local `var` is unchanged.
- **BREAKING:** code that returned or passed a non-`Copy` field by value (`fun f(u: &User): String = u.name`) is now rejected. Existing fixtures and tests that do this are rewritten to clone, borrow or compare the field.
- Not in scope: use-after-move and conditional-move analysis (step 7, `borrowck-moves`); implicit shared borrows, copy-out of `Copy` values from borrows, moving out of a borrow, and prefix `*` (step 3, `reference-access`); user-defined methods (step 4); custom `clone()` implementations (traits); `Copy` and `Clone` bounds on type parameters (generics); destructuring and partial moves.

## Capabilities

### New Capabilities
- `ownership`: which types are `Copy` or `Clone`, how the markers are checked, explicit cloning, what a move may take, and the immutability of top-level declarations. Later steps add use after move and reinitialization here.

### Modified Capabilities
- `syntax`: struct and enum declarations accept a `: Copy` or `: Clone` marker, and `var` is a local-only binding.
- `compilation-and-abi`: exported structs and enums, and nominal type references in exported signatures, record their marker, and the ABI hash covers it. Exported top-level values no longer record a `var` flag.

## Impact

- `crates/prism-ast`: `StructDecl` and `EnumDecl` gain `marker: Option<TypeMarker>` (`Copy` or `Clone`). `TopLevelVarDecl` loses `is_var` and keeps its name, since a `val` is a read-only variable.
- `crates/prism-parser`: parses the marker after the type parameters and the shorthand parameter list, and rejects a top-level `var` with a targeted `E2001` message. `x.clone()` and `x?.clone()` already parse as calls on a member access.
- `crates/prism-abi`: `ExportedSymbol::Struct` and `Enum` and `AbiType::Nominal` gain the marker, the name binder returns it, the hash covers it, and `ExportedSymbol::Var` loses `is_var`. The built-in `Range` gets its own ABI node instead of being recorded as an unbound name, so an imported `Range` field stays `Copy`. Every exported ABI hash changes once, because the hash format changes.
- `crates/prism-typechecker`: `Copy` and `Clone` predicates over `Type` and the type table, the structural check (`E4016`), the built-in `clone()` member (`E4018`), and the move-source check at consuming sites (`E4017`).
- `crates/prism-cli`: renders `E4016`, `E4017` and `E4018` with help lines, and prints markers in ABI dumps. New fixtures under `tests/fixtures/ownership/` and `syntax/`, and new Rust ABI scenario tests. About ten existing fixtures and `prism-typechecker` regression tests that return a `String` field by value are rewritten to clone, borrow or compare it instead.
- `openspec/specs/diagnostics/spec.md`: `E4016`, `E4017` and `E4018` are added to the catalog. The catalog is not a requirement, so it is edited directly, as past changes did.
- `docs/architecture.md`: §4 records what `clone()` does at runtime (field-wise, allocating for `String` and `Heap<T>`) as planned semantics awaiting code generation. On archive, the implemented §3 requirements ("Copy Types", "No Mutable Top-Level State" except the borrow-of-a-`val` scenario, and the field/`Heap`/top-level parts of "Moves take whole values only") move to `openspec/specs/ownership`.
