# Design

## Context

`Type::from_ast` and `Type::parse_str` (`crates/prism-typechecker/src/types.rs`) are context-free: they have no access to the symbol table, so every non-primitive name becomes `Type::Struct { name, fields: [] }`. Struct and enum declarations get their full member lists only on their own `Symbol` (`SymbolKind::Struct` / `SymbolKind::Enum`), and only a constructor call (`Point(1, 2)`) returns a `Type` built from that symbol, with the fields filled in.

The checker reads members in four places:

1. `infer_member_access` reads fields and enum variants (this is where `E4006` fires for `user.name`)
2. `check_assignment` handles field writes (`user.name = n`)
3. `infer_unary_op` / `BorrowMut` checks `val` fields under `&mut obj.field`
4. `infer_when_expr` checks exhaustiveness. It already special-cases `Struct { name }` by looking `name` up in the environment, so enum-typed parameters work there and nowhere else.

Struct field types are themselves lowered by `from_ast` when the struct is registered, so a struct-typed field (`val user: User`) also has an empty field list. Pass 1 registers declarations in source order, so a field may name a struct declared later in the file, and structs may be recursive through `Heap<T>`.

## Goals / Non-Goals

**Goals:**
- One resolution rule, applied wherever the checker reads a nominal type's members, that makes all annotated forms behave like constructor-built values.
- Keep `Type` and the ABI string format unchanged, so ABI hashes and downstream crates are unaffected.

**Non-Goals:**
- Rejecting unknown type names in annotations (e.g. reporting `E4001` for `val x: Usr`). `examples/demo/models/models.prism` uses `UInt64`, which is not a primitive yet, and generic type parameters (`type_params` on declarations) also reach the same fallback. Validating type names needs a builtin type list and generic scoping, which is a separate change.
- Qualified type paths (`models.User`). Imports bring ABI symbols in by bare name today, and that stays the same.
- Calling a struct-typed *value* (`p()` where `p: Point`). It still goes through the constructor branch of `infer_call_expr`. This is a pre-existing quirk, not caused by this bug.

## Decisions

### D1: Resolve nominal types lazily where members are read, not eagerly while lowering

Add `TypeChecker::resolve_nominal(&self, ty: &Type) -> Type`: given `Type::Struct { name, fields }` with empty `fields`, look `name` up in the type namespace (D2). If a struct or enum is found, return that symbol's full type; otherwise return `ty` unchanged. Every other type passes through untouched. The four member-reading sites call it on the type left after peeling `&`/`&mut`/`Heap`/`?`.

*Alternative considered: eager resolution.* Thread the environment into lowering (`lower_type(&ast) -> Type`) and substitute full member lists at annotation time. Rejected for three reasons:
- It only resolves one level: field types stored on struct symbols are lowered during Pass 1 before later declarations exist, so `order.user.name` would still need lazy resolution at the second hop.
- Recursive types (`Heap<Expr>` inside `Expr`) would need cycle guards to avoid infinite expansion.
- `Type` values would carry full copies of member lists, which makes `PartialEq` on `Type` compare field lists and could break the `==` checks scattered through the checker.

Lazy resolution has none of these problems and matches the `when` exhaustiveness code that already works this way.

An empty-fields `Struct` is still ambiguous with a declared `struct Empty {}`, but resolving it simply returns the same empty struct, so the ambiguity is harmless.

### D2: Type-namespace lookup that skips value symbols

Add `ScopeStack::lookup_type(name)`, which searches the scopes from innermost outward and returns the first `Struct` or `Enum` symbol, ignoring variables and functions with the same name. Plain `lookup` would let a local `val User = ...` hide the struct `User` when resolving an annotation. `infer_when_expr` switches to the shared helper so there is one rule.

Imported structs are already registered as `SymbolKind::Struct` by `ingest_file_abi`, and their field types come from `parse_str` (also field-less), so the same lazy rule covers imported types and their nested fields with no ABI changes.

### D3: Missing exported return type gets its own code, `E4011 MissingReturnType`

The `E4004` reuse is a category error: nothing is being called. Options:
- (a) Keep `E4004` and special-case the CLI help text. Cheap, but the code itself still misclassifies the error, and tooling that keys on codes would be misled.
- (b) Reuse the ABI layer's error. The ABI stage only rejects *expression*-bodied exports, and the typechecker is also run without the ABI stage (unit tests, future LSP), so it needs its own diagnostic anyway.
- (c) **Chosen:** a new `TypeError::MissingReturnType { function_name, span }` with code `E4011`. It continues the `E40xx` sequence, gets a specific message ("exported function 'f' must declare an explicit return type"), and a CLI help line ("add an explicit return type such as ': Unit'; public and package functions must declare return types for ABI extraction").

Which programs are accepted is unchanged: the typechecker keeps rejecting *every* exported function without a return type, block-bodied ones included, as the `syntax` and `system-invariants` specs require. Note that the ABI extractor is looser (it silently treats a block-bodied export as `()`). That mismatch is out of scope here and listed under Open Questions.

## Risks / Trade-offs

- [Every member access repeats a hash lookup per scope level] → Scopes are shallow; this costs about the same as resolving the identifier in the first place.
- [Opaque unknown names still pass annotation checking silently] → Explicitly a non-goal; member access on them still reports `E4006`, so the error shows up at first use rather than at the annotation.
- [The new error code could collide with other in-flight work] → No open changes exist (`openspec list` is empty) and `E4011` is unused across the repo.

## Open Questions

- Should `prism-abi` reject block-bodied exported functions without a return type (matching the typechecker and the `syntax` spec), or should the specs relax to "block body implies `Unit`"? This doesn't affect this change's specs or tasks; it should be its own follow-up change.
