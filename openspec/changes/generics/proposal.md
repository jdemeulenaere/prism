# Proposal: generics

## Why

Milestone 2 needs generic code: the compose engine's `State<T>`, `mutableStateOf<T>(value: T): State<T>` and `remember<T>(calc: () -> T): T` (Phase 4), and soon a growable list type for UI children and real apps. Today the parser accepts `fun id<T>(x: T)` and `struct Box<T>`, but the type checker treats `T` as an unknown, opaque name, `Box<Int>` loses its argument, and nothing checks a generic body. This change, item 10 of "Then: the road to Milestone 2" in `ROADMAP.md` and the Phase 2 item "Generics: type parameter checking and monomorphization", makes type parameters real from the parser to the borrow checker. Generic functions, extensions, structs and enums are checked once, where they are declared, into polymorphic typed trees and MIR, and the ABI exports them exactly. Generating code for each instantiation (monomorphization) needs the backend and the artifact cache. It becomes its own change, `monomorphization`, after `runtime-strings-and-drops`.

## What Changes

- **Kotlin declaration syntax.** `fun <T> id(x: T): T`, `fun <T : Copy> twice(x: T): Pair<T, T>`, `fun <T> Box<T>.get(): &T`, `struct Box<T>(val v: T)`, `enum Option<T> { Some(T), None }`. **BREAKING:** the old placement `fun id<T>(...)` is now `E2001`, with a message pointing to `fun <T> id(...)`.
- **Explicit type arguments at call sites**, written as in Kotlin: `id<Int>(5)`, `Box<String?>(null)`, `mutableStateOf<String?>(null)`, `remember<Int> { 0 }`, `s.map<Int> { n -> n + 1 }`. After a name, a `<` starts a type-argument list only when a complete list closed by `>` follows and the next token is `(`, or `{` on the same line. Otherwise the `<` is a comparison. **BREAKING:** comparisons are non-associative, as in Kotlin, so `a < b < c` is `E2001`. It was always a type error.
- **Type argument inference**, local and bidirectional. Explicit arguments come first. Otherwise the expected type fixes what it can, the types of the other arguments give lower bounds that are joined (`pick(1, null)` gives `Int?`), and closure arguments are checked last against the solved parameter types, so `remember { 0 }` infers `T = Int`. A parameter that remains unsolved is `E4013`, as is a generic function used as a value.
- **Bounds `: Copy` and `: Clone` only, on function type parameters.** In a generic body, a value of an unbounded `T` can only be moved, borrowed, dropped, wrapped in `T?` or `Heap<T>`, or passed on. It is never implicitly copied (`E4017` stays) or cloned (`E4018`), and has no `==`, operators or members. `T : Copy` copies implicitly, and `T : Clone` (or `Copy`) allows `x.clone()`. A type argument that does not satisfy its bound is the new code `E4021` `UnsatisfiedBound`.
- **Conditional markers on generic types.** `struct Pair<A, B> : Copy { val a: A, val b: B }` is accepted. `Pair<Int, Int>` is `Copy`, `Pair<String, Int>` is `Clone` but not `Copy`, and `Pair<User, Int>` is neither when `User` has no marker. A type parameter conditions the marker only when it occurs in a field or payload type, so `struct Handle<T> : Copy { val id: Int }` is `Copy` for every `T`, which is what `State<T>` needs. **BREAKING:** the scenario that rejects `struct Box<T> : Copy { val v: T }` with `E4016` now accepts it.
- **Type parameters range over borrow-free types.** A type argument, written or inferred, must not contain a reference outside a function type. `id(&a)`, `Box<&Int>` and `Heap<&Int>` in an annotation are the new code `E4022` `BorrowInTypeArgument`, so a generic function that works on borrows writes `&T` in its signature. A function value that may hold a closure's captures cannot be passed where the callee's declared parameter type has a type parameter (outside a function type), because the generic body may store it. That widens `E5007`. With both rules, a generic body is borrow-checked once, and the elision rules give the same answer before and after substitution.
- **Generic extensions.** A receiver may carry type arguments (`fun <T> Box<T>.get(): &T`, `fun Box<Int>.sum(): Int`). Extensions are still keyed by the receiver's type constructor and name, so a file may declare only one `Box.sum`. Receivers that are a bare type parameter or a `Heap<...>` stay `E2001`.
- **Generic types are invariant**, `Heap<T>` keeps its rules, and `T?` with `T = U?` is `U?` (nullables flatten, as `T??` already does).
- **Arity.** A generic type written with the wrong number of type arguments (`Box`, `Box<Int, Int>`, `Heap<A, B>`, `Heap`), or a call with the wrong number of explicit type arguments (`id<Int, Int>(1)`, `f<Int>()` for a non-generic `f`), is the new code `E4020` `WrongTypeArgumentCount`. Today `Heap<A, B>` silently becomes an opaque name.
- **Polymorphic typed trees and MIR.** Type parameters are `Type::Parameter`, and nominal types carry their type arguments. Each function, struct and enum records its type parameters, and calls record their type arguments next to the callee's declared parameter types. MIR bodies keep a type-parameter value's drop, which `monomorphization` later resolves to drop glue or nothing. The dumps print `fun <T> id`, `main::id<Int>` and `Box<Int>`.
- **ABI.** Exported declarations record structured type parameters with their bounds, signature types refer to them by position (`AbiType::Parameter`, so a type parameter never binds to a struct of the same name), and struct and enum headers, and every nominal reference, record which type parameters condition their marker. Generic extensions get a Monomorphization Hash like generic functions.
- Not in scope: monomorphization and code generation (`monomorphization`, `codegen-core`); serializing generic bodies for importers; traits, trait bounds and `where` clauses; bounds on struct or enum type parameters; variance annotations; generic top-level `val`s, closures or function types; extensions on a bare `T`; explicit type arguments without a call (`Option<Int>.None`, `val f = id<Int>`); rejecting unknown type names (now unblocked, still its own item).

## Capabilities

### New Capabilities
- `generics`: type parameters and their scope, bounds, what a generic body may do with a type parameter, type arguments (explicit and inferred), the borrow-free rule, generic types and their markers, generic extensions, and the polymorphic representation in the typed tree and MIR.

### Modified Capabilities
- `syntax`: type parameter lists before the function name, explicit type arguments at call sites, non-associative comparisons, and generic extension receivers ("Extension Function Declarations" no longer rejects type arguments on a receiver).
- `ownership`: "Built-In Copy Types", "Built-In Clone Types" and "Marker Declarations Are Checked Structurally" cover bounded type parameters and conditional markers on generic types.
- `compilation-and-abi`: generic signatures in the ABI, marker conditions on nominal headers and references, and the Monomorphization Hash for generic extensions.
- `references`: "Capturing Closures Do Not Escape" adds passing a capturing closure where the callee's declared parameter type has a type parameter.
- `typed-tree` and `mir`: calls and constructions over generic declarations record type arguments, and bodies record their type parameters.

## Impact

- `crates/prism-ast`: `TypeParameter { name, bound, span }` replaces `Vec<String>` on functions, structs and enums. `ExprKind::Call` gains `type_arguments`.
- `crates/prism-parser`: `fun <T>` placement, bounds, speculative call type arguments, non-associative comparisons, generic receivers.
- `crates/prism-abi`: structured type parameters, `AbiType::Parameter`, marker conditions, and the hash over them. The `fun id<T>` tests move to `fun <T> id`.
- `crates/prism-typed-tree`: `Type::Parameter`, `Type::Struct`/`Type::Enum` with arguments, substitution, generic `TypeDef`s and conditional `is_copy`/`is_clone`, `type_parameters` on items and bodies, and `type_arguments` on calls, plus validator and dump updates.
- `crates/prism-typechecker` (mostly `checker.rs`): type parameter scopes, inference, bound, arity and borrow-free checks, generic constructions, variants, extensions and field access.
- `crates/prism-mir`: lowering, validator and dump over parameters and type arguments.
- `crates/prism-borrowck`: `Type::Parameter` in the place, loan and drop analyses, and the new `E5007` position.
- `crates/prism-cli`: renders `E4020` to `E4022` and the new helps. New fixtures go under `tests/fixtures/generics/`.
- `openspec/specs/diagnostics/spec.md`: `E4020`, `E4021` and `E4022` are added, and the `E2001`, `E4006`, `E4010`, `E4013`, `E4016`, `E4018` and `E5007` rows are widened.
- `ROADMAP.md`: item 10 is narrowed, and `monomorphization` is added after `runtime-strings-and-drops`. `docs/architecture.md` §1 gains an open decision on where instances live, and "Re-monomorphization on Generic Body Change" stays planned for `monomorphization`.
