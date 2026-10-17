# Design: value-semantics

## Context

See `proposal.md` for motivation. This change is step 2 of "the road to the borrow checker" in `ROADMAP.md`. The user agreed the sequence and the requirements it implements ("Single-Owner Value Semantics and RAII", "Copy Types" and "No Mutable Top-Level State" in `docs/architecture.md` §3, with their design notes) in an `/opsx:explore` session on 2026-10-17. On the same day, while this change was being proposed, the user:
- left four open questions to "the best design for long-term maintenance" (D2, D6, D8's error code, D9);
- decided to design explicit cloning now rather than defer it, choosing the name `clone()` and an opt-in `: Clone` marker that `: Copy` implies (D1, D5).

Current state:

- **No notion of `Copy`.** `Type` (`crates/prism-typechecker/src/types.rs`) has primitives, `Reference`, `Nullable`, `Heap`, `Struct(NominalId)`, `Enum(NominalId)`, `Opaque(String)`, `Fn`, `TypeParam` and `Error`. `TypeDef` holds struct fields or enum variants only. `Double` is an alias of `Float`. Any other undeclared name (`UInt64`) is `Opaque`.
- **No methods.** `x.f()` parses as a call whose callee is a member access, and `infer_call_expr` types it by inferring the callee, so today it can only call a function-typed field. Methods arrive with `extension-methods` (step 4).
- **Field access reads through `&`, `&mut` and `Heap<T>`** (`peel_indirection`), so `h.x` on a `Heap<Point>` is a field access. Indexing exists only on `String` and gives `Char`.
- **Value positions** are checked by `check_expr_at(expr, expected, Origin)` with origins Value, Return and Argument. Unannotated local and top-level initializers go through `infer_expr`.
- **Struct and enum declarations** (`crates/prism-parser/src/parser.rs`, `parse_struct` and `parse_enum`) take a name, optional `<T, …>` type parameters, then either a shorthand `( … )` parameter list (structs only) or a `{ … }` body. Nothing can follow the parameter list.
- **Top-level `var`** parses into `TopLevelVarDecl { is_var: true }`, and the ABI records `ExportedSymbol::Var { is_var }`. No fixture or example declares one.
- **ABI nominal references** (`AbiType::Nominal { kind, module, name, args }`) already record struct versus enum, so an importer can type a value whose module it does not import. Phase 1a reads each file's exported type names and kinds from its AST alone, and Phase 1b binds names using only those outputs. A name that binds to nothing, `Range` included, is recorded `Unbound` and comes back to an importer as `Opaque`.
- **Existing tests read `String` fields by value.** About ten fixtures and `prism-typechecker` regression tests do so (`fun f(u: &User): String = u.name`, `u?.name`, `o.user.name`). They exercise field typing, not moves, but this change rejects them.

## Goals / Non-Goals

**Goals:**
- `Copy` and `Clone` predicates in the type checker, used by the declaration check, `clone()` and the move-source check, that give the same answer for local and imported types.
- Markers flow through the ABI exactly, so files still compile independently and builds stay hermetic.
- With whole-value moves, every non-`Copy` field stays reachable: by borrowing it, or by cloning it when its type is `Clone`.
- Every new scenario has a `.prism` fixture or, for ABI hashing, a Rust ABI scenario test, and `E4016`, `E4017` and `E4018` each have a fixture.

**Non-Goals:**
- Use after move, conditional and loop moves, reinitialization (step 7). After this change `val b = a; take(a)` for `a: String` still typechecks. Rejecting that second use is the borrow checker's job.
- Moving out of a borrow as a whole (`take(u)` for `u: &User`, still `E4004` today), copy-out of `Copy` values from borrows, and implicit shared borrows (step 3).
- Moves of captured variables inside closures. A capture is treated as a local here. Since closures capture by inferred borrow (step 8), such a move becomes a move out of a borrow in steps 3 and 8.
- User-defined `clone()` bodies, and `Copy`/`Clone` bounds on type parameters. Both need traits or generics.
- Code generation for `clone()`. Its runtime meaning is recorded in `docs/architecture.md` §4 as planned semantics.
- Rejecting borrows in enum payloads. The parser rejects them only in struct fields today (see Risks).

## Decisions

Decisions marked **(user)** were made by the user, in the exploration session, in the conversation that added cloning, or by leaving the choice to the long-term option on 2026-10-17. The others follow from them or from existing rules, and are open to review here.

### D1. `Copy` and `Clone` are opt-in markers, and `Copy` implies `Clone` (user)

A struct or enum is `Copy` only with `: Copy`, and `Clone` only with `: Clone` or `: Copy`.

- **`Copy`** means implicit duplication by a bitwise copy that runs no user code and never allocates, so it can stay invisible.
- **`Clone`** means explicit duplication with `.clone()`, which may allocate (a `String` buffer, a `Heap<T>` box), so it is always written out. That is why `String` is `Clone` but not `Copy`: an implicit deep copy would be a hidden allocation.
- **Both are opt-in.** Each is a deliberate API promise, visible in the declaration and the ABI. Adding a field never silently changes what a public type can do, and a unique token, or a type that will later own a resource or have a destructor, is never duplicable by accident. Structural derivation (Swift's model) was rejected: it would need opt-out markers later. Making every type with `Clone` fields cloneable automatically was rejected for the same reason.
- **`Copy` implies `Clone`.** A `Copy` value's clone is its copy. Rust requires writing both (`#[derive(Clone, Copy)]`), which buys nothing without custom clone bodies.

Because markers are declared, both predicates are **shallow**. A nominal type is `Copy` or `Clone` iff its declaration says so, whatever its fields contain, so neither the predicates nor the structural check (D4) recurse through type definitions or need a cycle guard.

### D2. Grammar: Kotlin supertype position, one marker (user)

```
struct_decl = "struct" Ident type_params? ( "(" params ")" marker? | marker? "{" fields "}" )
enum_decl   = "enum" Ident type_params? marker? "{" variants "}"
marker      = ":" ( "Copy" | "Clone" )
```

- **Position:** as in Kotlin (`class Foo<T>(val x: T) : Bar { … }`), after the type parameters and the shorthand parameter list, before the body. When traits exist, this is where a marker trait list would go.
- **Exactly one marker name.** The parser accepts the identifier `Copy` or `Clone` and nothing else, so `: Eq`, `: Copy, Clone` (redundant, D1), a repeated name and an empty `:` are `E2001`, with a message saying that exactly one of the two may follow and that `Copy` implies `Clone`. Neither name is a keyword: they are matched as identifiers in this one position. A struct named `Copy` and a parameter named `Clone` still parse, and in marker position the names always mean the markers, even if a user type of that name exists.
- **AST:** `StructDecl` and `EnumDecl` gain `marker: Option<TypeMarker>`, with `enum TypeMarker { Copy, Clone }` in `prism-ast`, reused by `prism-abi`.
- **Rejected:** parsing a general comma-separated supertype list now and rejecting unknown names in the type checker. `ROADMAP.md` already expects trait conformances to need their own block (`extension T : P`), because methods are extension functions, so a general list would invent rules for names that may never appear there. Widening the marker into a list later is additive and breaks no program.

### D3. The `Copy` and `Clone` predicates and the built-in tables (user, with additions)

`Checker::is_copy(&Type)` and `Checker::is_clone(&Type)`:

| Type | `Copy` | `Clone` |
|---|---|---|
| `Int`, `Float`, `Bool`, `Char`, `Unit`, `Nothing`, built-in `Range` | yes (user) | yes |
| `&T` | yes (user) | yes |
| `String` | no (user) | yes (user) |
| `T?` | iff `T` (user) | iff `T` is `Clone` (user) |
| `Heap<T>` | no (user) | iff `T` is `Clone` (user) |
| `Struct(id)`, `Enum(id)` | iff marked `Copy` | iff marked `Copy` or `Clone` |
| `&mut T`, `Fn` | no (user) | no (user) |
| `TypeParam`, `Opaque` | no | no |
| `Error` | yes | yes, so an earlier error is not reported twice |

- **Function types are not `Clone`.** A closure holds inferred borrows of its captures (step 8), and a clone would duplicate those borrows, including `&mut` ones.
- **Type parameters are neither** until bounds exist, as `ROADMAP.md` already states for generics.
- **Unknown names (`Opaque`) are neither.** Duplicating a value is a promise, and nothing has made it for an undeclared name. When unknown names are rejected (`E4001`, a ROADMAP item), this row disappears. As a side effect, `UInt64` in `examples/demo/models` cannot sit in a marked struct until sized integers exist. The example declares no marker, so it is unaffected.
- **One marker map.** The checker keeps a single `NominalId -> Option<TypeMarker>` map, filled from the file's own declarations, from its imports' exported structs and enums, and from every nominal reference in an ingested ABI. The last source covers a type whose module is not imported, so it has no `TypeDef` (D6). `TypeDef` itself is unchanged. `Type::Struct` and `Type::Enum` stay identity-only. Putting the marker on `Type` itself was rejected: two values of the same type could then disagree, and equality would have to ignore a field. Storing it on `TypeDef` plus a side map for unimported types was rejected as two sources of truth.

### D4. Structural check of marker declarations, `E4016` (user)

After every struct and enum of the file has been registered and the imports loaded, each marked declaration checks every field or payload type: with `is_copy` for `: Copy`, with `is_clone` for `: Clone`. It reports `InvalidMarkerDeclaration` (`E4016`) once per offending field, naming the marker, the field (or the variant and 1-based payload position) and its type. The help line says that a `Copy` type can only contain `Copy` types, or a `Clone` type only `Clone` types.

It runs in the defining file's Phase 2 only. Importers trust the ABI marker, since a file with an invalid declaration fails its own check, and with it the build. Because the predicates are shallow (D1), the order of declarations and mutually referring types need no special handling.

### D5. Explicit `clone()`, `E4018` (user)

- **Name: `clone`, not `copy` (user).** Kotlin data classes have `copy()`: a shallow copy with named overrides (`user.copy(name = "Bob")`). Giving `copy()` a deep meaning would mislead Kotlin users. A `.copy()` on a type that is already `Copy` would also read oddly. `clone` also keeps `copy(...)` free for a possible struct-update feature.
- **A built-in member, not a library function.** Methods do not exist yet (step 4), and even then a user-written `fun T.clone()` could not be called generically without traits. `clone` is therefore a compiler-known member of every `Clone` type, typed in `infer_call_expr`: a call whose callee is a member access (or safe navigation) named `clone`.
- **Typing.** The receiver is read in place: it is a member-access base, so it is never consumed (D8). The checker peels every `&`/`&mut` layer from the receiver's type (not `Heap<T>`, which is an owned value), giving `T`. If `T` is `Clone` the call has type `T` and is a temporary, so its result can be moved. `x?.clone()` on `T?` gives `T?`. Since `T?` is `Clone` when `T` is, `x.clone()` on a `T?` also gives `T?`.
- **Errors.** If `T` is not `Clone`, the call reports `NotCloneable` (`E4018`), naming `T`. For a struct or enum, the help suggests adding `: Clone` to its declaration. For a type parameter, it says bounds are not supported yet. For a function type, it says closures cannot be cloned. Arguments to `clone` report `E4004`, as for any arity mismatch.
- **A field named `clone` wins over the built-in (user).** When the receiver's type (after peeling borrows and `Heap<T>`) has a field named `clone`, `x.clone()` reads and calls that field, as today. This matches Kotlin, where members, including a property of function type, take precedence over extensions, and the built-in behaves like a compiler-provided extension. Step 4 (`extension-methods`) can apply the same rule to every field-versus-extension clash, and must reject a user extension named `clone` on a `Clone` type. Rejected: "the built-in always wins". The parser drops parentheses, so `(x.clone)()` cannot be told apart from `x.clone()`, and preserving them would add an expression node for an edge case; without it, such a field could not be called at all. The cost is that a `Clone` type with a field named `clone` cannot use `.clone()`, which is rare.
- **Semantics: field-wise.** A user type's clone clones each field or payload, and a built-in's clone is defined in the runtime semantics (`docs/architecture.md` §4). No user code runs until traits allow custom clone bodies. The marker then becomes the "derive it" form, and a custom body would replace it.
- **Rejected:** a free function `clone(x)`, which is less Kotlin-like and would later collide with methods; and deferring cloning until traits, which would leave whole-value moves with no way to get an owned copy of a field.

### D6. Markers in the ABI, on declarations and references (user)

- `ExportedSymbol::Struct` and `ExportedSymbol::Enum` gain the marker, hashed as one byte (none, `Clone`, `Copy`).
- `AbiType::Nominal` gains the marker, which is hashed with the reference. Phase 1a's per-file output becomes name → (kind, marker), still read from the AST alone (the marker is syntactic), and the `NameBinder` returns it with the module. No body is ever read, and import cycles still work, because 1b reads only 1a outputs.
- **Consequence:** changing a type's marker changes the ABI hash of every file whose exported signatures mention the type. That is correct, because what those signatures allow importers to do (read a field by value, clone it) changes.
- **Rejected:** (a) reporting `E4014` (import the defining module) whenever a marker is needed and unknown, which would make an import change whether unrelated code compiles; (b) treating an unknown marker as "none", which gives false errors for types that are `Copy` or `Clone`; (c) relaxing the hermetic-builds invariant to read transitive ABIs, already rejected in `typechecker-soundness` (D9).
- Calling `clone()` on a value whose type's module is not imported is accepted (spec "Field of a type from an unimported module"). It names no member of the type, so `E4014` does not apply, and the marker comes from the reference.

### D7. The built-in `Range` in the ABI

Today an exported `Range` is recorded `Unbound` and comes back to an importer as an `Opaque("Range")` type, distinct from the built-in. With D3 that would make an imported `Range` field move-only, so the ABI must record it exactly. When a name `Range` binds to neither the file's own types nor an import, Phase 1b records a new `AbiType::Range` node (its own hash tag), which `Type::from_abi` lowers to `NominalId::builtin_range()`. A user-declared or imported `Range` keeps binding first, matching `bind_type_name` in the checker. A general `AbiType::Builtin(name)` was rejected: `Range` is the only built-in nominal type, and a closed set is simpler to hash.

### D8. Moves take whole values only, `E4017` (user)

**Consuming positions** (spec "Consuming Positions"): local, parameter-default and top-level `val` initializers, assigned values, by-value arguments, struct constructor fields and enum payloads, and returned values (`return` operands, expression bodies, block tails feeding a return, closure results). A consuming position passes through `if`/`when` branches, block tails, parentheses and both operands of `?:`. Everything else reads in place: operator operands, conditions, `when` subjects, `&`/`&mut` operands, member-access bases (so `clone()` receivers), callees, index targets, the `for` iterable and discarded expression statements. A discarded statement has no destination, so nothing moves.

**Implementation:** one function, `check_move_source(expr, ty)`, runs after an expression in a consuming position is typed. It recurses through the pass-through forms and classifies leaves:

- an identifier naming a local or parameter (a whole owned value): accepted;
- an identifier naming a top-level `val` whose type is not `Copy`: rejected. A top-level `fun` name yields a fresh function value and is accepted;
- a member access, safe navigation or index whose result type is not `Copy`: rejected, whatever the base (owned, borrowed, `Heap<T>` or a temporary);
- anything else (calls, including `clone()`, constructors, variants, literals, closures, operators): a temporary, accepted.

A rejection is `CannotMoveOut` (`E4017`): "cannot move out of {source}", where the source is "field `name` of `u`" or "top-level value `GREETING`". The help always suggests a borrow (`&u.name`). When the type is `Clone` it also suggests a clone (`u.name.clone()`, or `u.nick?.clone()` for a safe navigation or nullable field). A value of the error type is never reported, so `E4014` and `E4006` do not cascade.

- **Fields of temporaries are rejected too** (`make().name`). "A value is either whole or gone" then holds without exceptions, and a temporary's other fields never need a partial drop, which matters once destructors exist (Rust forbids it for types with `Drop`, E0509). `make().name.clone()` is the explicit form. Allowing the move later breaks no program, like partial moves out of locals (both ROADMAP relaxations).
- **`?:` and safe navigation consume.** `u.nick ?: "anon"` moves `u.nick` when it is not `null`, so it is rejected like any other field move. `u.nick?.clone() ?: "anon"` is the explicit form, and the help says so.
- **One code for every move-out source (user).** Step 3 (`reference-access`) adds the source "borrow `u`" to the same `E4017` rather than taking a new code, as Rust uses one E0507 for every place behind a reference. The fixes are the same (borrow, clone, later destructure or copy out), and tools key on one code. The message format above is written so that step 3 only adds a source kind.
- **Why the type checker:** every case is decided by the expression's shape and its type, without control flow, so it is an `E4xxx` rule (`docs/architecture.md` §3). It also keeps `borrowck-moves` (step 7) small: it tracks whole locals only, because no other source can be moved.

### D9. Top-level `var` is removed in the parser (user)

`parse_declaration` accepts `val` at top level and rejects `var` with `E2001`: "top-level 'var' is not allowed: top-level declarations are immutable; keep mutable state in locals and parameters, or in a state handle in UI code". It points at `var` and covers every visibility. `TopLevelVarDecl.is_var` and `ExportedSymbol::Var.is_var` are removed, so neither the AST nor the ABI can represent a mutable global. The type names stay (`TopLevelVarDecl`, `ExportedSymbol::Var`), because a `val` is a read-only variable, as Kotlin calls it. The `ImmutabilityViolation` path for assigning a top-level binding still fires (`E4003`), because a top-level `val` is immutable.

Rejected: keeping the grammar and adding a type-checker code (`MutableTopLevelState`). The AST and ABI would keep a flag that no valid program sets, and Phase 1 would run before the rejection. Rejecting it in the parser follows the precedent of borrowed struct fields (`syntax`, "Struct Field Borrow Prohibition").

### D10. Error codes

`E4016` (`InvalidMarkerDeclaration`), `E4017` (`CannotMoveOut`) and `E4018` (`NotCloneable`) are the next free `E4xxx` codes, per the diagnostics catalog's rule for new error kinds.
- None reuses `E4002`: a declaration that cannot carry its marker, a move from the wrong place and a clone of a non-`Clone` type are not type mismatches, and each needs its own help line.
- `E4018` does not reuse `E4006` (`UnknownField`), because the fix is to add a marker, not to fix a name.
- One `E4016` for both markers, because the check and its fix are the same, with the marker named in the message.

### D11. Existing tests keep their scenarios

Fixtures and regression tests that read a non-`Copy` field by value to test field typing are rewritten so they still exercise their scenario without moving: `u.name.clone()` where the value is returned or passed on, `u?.name?.clone()` for safe navigation, or `&u.name` and comparisons where the clone would obscure the scenario. Their names and expectations stay. Changing their expectations to `E4017` was rejected: they pin field typing, and the new rule has its own fixtures.

## Risks / Trade-offs

- [Kotlin habits such as `fun name(u: &User): String = u.name` and `u.nick ?: "anon"` stop compiling] → `E4017`'s help shows both fixes, `&u.name` and `u.name.clone()` (`u.nick?.clone() ?: "anon"`). The allocation that Kotlin hides behind a shared reference is now visible, which is the point of the model (`docs/architecture.md` §3 design notes, "Whole-value moves only").
- [`clone()` may be overused where a borrow would do, allocating needlessly] → borrows are always suggested first. Once the borrow checker exists, returning `&String` from a `&User` parameter is checked for soundness (elision rule 1, step 9).
- [Until step 7, a moved local can still be used again] → expected: this change rejects only flow-insensitive cases, and the non-goals say so.
- [A built-in `clone` member before methods exist] → its rules are written so step 4 only has to forbid a conflicting extension, and traits can later turn the marker into a derived implementation without changing call sites.
- [Every exported ABI hash changes once, because the hash format changes] → there is no `.abi` cache yet, so nothing is invalidated in practice.
- [A dependency's marker change cascades into importers' ABI hashes] → intended (D6), and limited to files whose exported signatures mention the type.
- [Enum payloads may hold borrows today (`enum A { X(&Int) }`), and with `&T` being `Copy`, `enum A : Copy { X(&Int) }` is accepted] → a pre-existing gap in the zero-cost memory-safety invariant's parse-time borrow check, which covers struct fields only. It is listed as a ROADMAP follow-up and needs no change to this design.
- [Opaque names being neither `Copy` nor `Clone` rejects markers over not-yet-built-in types like `UInt64`] → accepted until sized integers are built in. Treating unknown names as duplicable would be a silent promise.

## Migration Plan

No external users. In-repo: the fixtures and tests listed in D11 are rewritten in the same commit as the check that rejects them. Rollback is a revert.

## Open Questions

- The exact help-line wording of `E4016`, `E4017` and `E4018` can be tuned during implementation, provided `E4017` keeps the "cannot move out of {source}" shape that step 3 extends.
