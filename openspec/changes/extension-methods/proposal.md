# Proposal: extension-methods

## Why

Prism has no methods. `x.f()` only calls a function-typed field or the built-in `clone()`, so every operation on a struct is a free function (`updateName(&mut user, "x")`), built-in types gain no operations at all, and the receiver forms that the elision rules and the borrow checker are designed around (`fun`, `mut fun`, `consuming fun`) do not exist yet. This change, step 4 of "the road to the borrow checker" in `ROADMAP.md`, adds methods as extension functions only, as agreed in the `/opsx:explore` session of 2026-10-17 (`docs/architecture.md` §6 and the §3 scenario "Method receivers borrow implicitly"). The typed tree (step 5) can then record every receiver borrow as an explicit adjustment, and the borrow checker (steps 7 to 9) can rely on receiver modes being in the ABI.

## What Changes

- **Extension functions are the only methods.** `fun T.name(...)` declares a method on `T` in any file, with the usual visibility modifiers. Structs and enums keep declaring data only. A call `value.name(...)` resolves statically from the receiver's static type, after looking through any number of `&`, `&mut` and `Heap<T>` layers. Built-in types (`Int`, `String`, `Range`, ...) gain methods the same way (`fun Int.squared(): Int = this * this`).
- **Receiver modes.** A plain `fun` borrows the receiver shared, `mut fun` borrows it mutably and `consuming fun` takes it by value. `this` has type `&T`, `&mut T` or `T` accordingly. `mut` is already a keyword; `consuming` is a contextual keyword, a modifier only directly before `fun`.
- **Receivers are borrowed implicitly at the call site**, including for `mut fun`, the one exception to explicit `&mut`. The receiver of a `mut fun` must be a mutable place (a `var`, a writable field path, a place reached through `&mut`, or a temporary), otherwise `E4003`. The receiver of a `consuming fun` is a consuming position, so the existing move rules apply (`E4017` for a field, a top-level `val` or a borrow of a non-`Copy` value).
- **Implicit `this`.** In an extension body, a name that is not a local or parameter resolves to a field of the receiver, and a bare call to an extension on the receiver type, before top-level declarations and imports. `this` outside an extension is `E4001`.
- **Resolution through imports.** An extension is visible when declared in the file or exported by a direct import. A file's own extension shadows imported ones, and two imports exporting a matching extension are ambiguous at use (`E4012`). Calling an imported extension does not require importing the receiver type's module.
- **Members win.** A field takes precedence over an extension of the same name, as a field named `clone` does over the built-in `clone()`, and the built-in `clone()` of a `Clone` type wins over an extension. Declaring an extension named like a field of its receiver struct, or named `clone` on a `Clone` type, is rejected with `E4010`, since it could never be called. Two extensions with the same receiver and name in one file are `E4010`.
- **Safe-navigation calls.** `u?.greet()` calls the extension on a non-null receiver and gives `R?` otherwise. A chain of `?.` field accesses over a place (`t?.lead?.greet()`) is borrowed where it lives rather than moved into a temporary. Calling with `.` on a nullable receiver is `E4006`, with help suggesting `?.`.
- **ABI.** An exported extension is recorded as a function signature with its receiver type and mode, both covered by the ABI hash. Struct and enum records never depend on extensions, so adding an extension in another file never changes the receiver type's file hash.
- No new error codes: `E4001`, `E4003`, `E4006`, `E4010`, `E4012` and `E4017` are widened, and their catalog rows are updated.
- Not in scope: extensions on nullable, reference, function or generic receiver types (`fun String?.orEmpty()`, `fun <T> Heap<T>.x()`, which comes with `generics`); extension properties; method references (`u::greet`) or reading an extension as a value; static and companion functions (`User.create()`); trait conformances; field visibility; two-phase borrows and use after move, which are borrow-checker work (steps 7 and 8).

## Capabilities

### New Capabilities
- `extension-functions`: how methods are declared as extension functions, how a call `value.name(...)` resolves (members first, own file, then imports), implicit `this` in extension bodies, the checks on extension declarations, and safe-navigation calls.

### Modified Capabilities
- `syntax`: extension declarations `fun T.name(...)`, the `mut` and `consuming` receiver modifiers, the `this` expression, and `consuming` as a contextual keyword.
- `references`: receivers are borrowed implicitly by their mode, through any number of reference layers, and "Explicit Mutable Borrows at Call Sites" gains the receiver exception.
- `ownership`: the receiver of a `consuming fun` call is a consuming position, and "Explicit Clone" says how an extension named `clone` interacts with the built-in.
- `compilation-and-abi`: exported extensions are recorded with their receiver type and mode, and the receiver type's own record is unaffected.

## Impact

- `crates/prism-ast`: `FunctionDecl.receiver: Option<Receiver>` with a `ReceiverMode` (`Shared`, `Mutable`, `Consuming`), and `ExprKind::This`.
- `crates/prism-lexer`: none (`this` and `mut` are already keywords).
- `crates/prism-parser`: `parse_declaration` reads the `mut`/`consuming` modifier, `parse_function` reads a receiver before the name, and `parse_prefix`/`starts_expr` accept `this`; targeted `E2001` messages for misplaced modifiers and unsupported receiver types.
- `crates/prism-abi`: `ExportedSymbol::Function` gains a receiver; `sort_key`, `calculate_hash` and the span-free body hash cover it.
- `crates/prism-typechecker`: an extension table separate from the name scopes, own-file and imported; call resolution in `infer_call_expr`; receiver adjustment (auto-deref, borrow, reborrow, move, copy-out) with mode checks; `this` binding and implicit `this` resolution; declaration checks; inference of unannotated private extensions keyed by receiver and name.
- `crates/prism-cli`: renders the widened diagnostics and their help lines. New fixtures go under `tests/fixtures/extension-functions/`, plus `syntax/`, `references/`, `ownership/` and `compilation-and-abi/` for the modified specs.
- `openspec/specs/diagnostics/spec.md`: the `E4001`, `E4003`, `E4006`, `E4010`, `E4012` and `E4017` rows are widened (the catalog is not a requirement, so it is edited directly).
- `docs/architecture.md`: §4 gains the planned evaluation order of method calls. On archive, §6 and the §3 scenario "Method receivers borrow implicitly" move into the specs; the use-after-move half of "Consuming method moves the receiver" moves to §3 for `borrowck-moves`.
- `examples/`: `demo/models` can turn `updateName` and `rename` into `mut fun User.rename(...)`.
