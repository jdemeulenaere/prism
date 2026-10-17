# Design: extension-methods

## Context

See `proposal.md` for motivation. This change is step 4 of "the road to the borrow checker" in `ROADMAP.md`. The user agreed its core in the `/opsx:explore` session of 2026-10-17: `docs/architecture.md` §6 (extensions as the only methods, declarable in any file, `fun` / `mut fun` / `consuming fun`, implicit `this`, static dispatch, `E4012` for ambiguous imports, own-file shadowing) and the §3 scenario "Method receivers borrow implicitly". `value-semantics` left two items for it: a field takes precedence over an extension of the same name, and a user extension named `clone` on a `Clone` type is rejected. The user was not available while this change was proposed, so every other question is decided here and marked **(autonomous)**, and logged in `docs/decisions-for-review.md`.

Current state:

- **Parser** (`crates/prism-parser/src/parser.rs`). `parse_declaration` reads a visibility, then `compose`, then dispatches on `fun`/`struct`/`enum`/`val`. `parse_function` expects `fun`, an identifier, optional type parameters (`fun f<T>(...)`), then `(`. `fun User.greet()` fails at the `.`. `this` and `mut` are lexer keywords (`TokenKind::This`, `TokenKind::Mut`); `this` is not accepted anywhere by the parser, and `mut` only after `&`. `compose` is a soft keyword, handled by `expect_ident` and `parse_atom`.
- **AST** (`crates/prism-ast`). `FunctionDecl` has `visibility`, `is_compose`, `name`, `type_params`, `params`, `return_type` and `body`. There is no receiver and no `this` expression.
- **ABI** (`crates/prism-abi/src/signature.rs`). `ExportedSymbol::Function` has no receiver. Symbols are sorted by `sort_key`, `(kind, name)`, with a stable sort, so two exported functions with the same name would hash in declaration order. Struct and enum records carry only their fields or variants and marker.
- **Type checker** (`crates/prism-typechecker/src/checker.rs`). `register_function_decl` defines every function by name in module scope (`E4010` on a duplicate name), and `ingest_file_abi` defines every exported function by name in the import layer of `ScopeStack`. Inference of unannotated private declarations (`inference`, `ensure_inferred`) is keyed by name. `infer_call_expr` has one method-like branch: `x.clone()` calls `infer_clone_call` unless `has_field(x, "clone")`; any other `x.f()` infers `x.f` as a field and calls it. `check_argument` and `coerce` implement the implicit argument borrows of `reference-access`, returning an `Adjustment` that is not stored. `is_place`, `infer_place` and `contents_writable` decide places and their mutability, and `check_move_source` reports `E4017`.
- **Imports.** `ScopeStack::lookup_import` keeps one candidate per exporting module, and a use with several candidates reports `E4012`. Members of a type whose module is not imported report `E4014` (`reject_unimported_members`).

## Goals / Non-Goals

**Goals:**
- One method mechanism for user and built-in types, resolved statically from the receiver's type, with nothing written at the call site for any receiver mode.
- Every receiver pass (auto-deref, borrow, reborrow, move, copy-out) is decided at one point in the checker and classified, so the typed tree (step 5) records it rather than re-deriving it.
- Exported extensions are extractable in Phase 1 from the AST alone, and adding one never changes the ABI of the receiver type's file.
- Every new scenario has a `.prism` fixture.

**Non-Goals:**
- Use after move through a `consuming` receiver, borrow conflicts, two-phase receiver borrows (`v.push(v.size())`) and returned-borrow lifetimes (elision rule 2). These are steps 7 to 9; the type checker accepts what they will check.
- Extensions on nullable (`fun String?.orEmpty()`), reference, function or generic receiver types; generic receivers come with `generics` (step 10).
- Extension properties (`val User.fullName`), method references (`u::greet`), static and companion functions (`User.create()`), trait conformances and field visibility.
- Overloading: one extension per receiver type and name in a file.

## Decisions

Decisions marked **(user)** were made by the user in the exploration session; **(autonomous)** ones were made while proposing this change, on the user's behalf, and are open to review. The others follow from these or from existing rules.

### D1. Extensions are the only methods, declarable in any file (user)

`fun T.name(...)` declares a method on `T`. Structs and enums declare data only (fields, variants, marker). An extension may be declared in any file, with any visibility, and is called as `value.name(...)`. Dispatch is static: the receiver's static type picks the extension at compile time, and there is no virtual call.

Alternatives (exploration): member functions inside struct bodies, which would need Kotlin's member-versus-extension precedence and put methods into the struct's ABI; and Go's rule that methods live in the type's package, which mainly protects trait coherence and can be added for conformances when traits exist.

### D2. Receiver modes are modifiers before `fun` (user), in a fixed order (autonomous)

`fun T.m()` borrows the receiver shared, `mut fun T.m()` mutably, and `consuming fun T.m()` takes it by value. Inside the body, `this` has type `&T`, `&mut T` or `T`. The modifiers keep the common case unmarked and follow Swift's `mutating` / `consuming`. Rust's explicit `&self` / `&mut self` / `self` parameter was the alternative (exploration).

Grammar **(autonomous)**: `[visibility] [compose] [mut | consuming] fun Receiver.name`. `mut` is already a hard keyword. `consuming` becomes a contextual keyword exactly like `compose`: a modifier only directly before `fun` at the start of a declaration, an identifier everywhere else, so no existing name breaks. The mode lives in the receiver (`FunctionDecl.receiver: Option<Receiver { ty, mode, span }>`, `ReceiverMode::{Shared, Mutable, Consuming}`), so a mode without a receiver cannot be represented, and the parser rejects `mut fun f()` with a message pointing at `mut fun T.name()`. Rejected: Kotlin's free modifier order, which buys nothing with three modifiers and makes the formatter's job harder; and hard-keyword `consuming`, which would break any variable named `consuming` for no grammatical gain.

### D3. A receiver is a single type name for now (autonomous)

The receiver is parsed as one identifier followed by `.` and the method name. Receivers with type arguments (`Heap<User>`), `?`, `&`/`&mut` or parenthesized types are `E2001`, each with a targeted message: type arguments wait for `generics`; a nullable receiver should be declared on the non-null type and called with `?.` (D10); a reference receiver is expressed by the mode. Because type parameters already follow a function's name (`fun f<T>()`), `fun Heap<User>.size()` is detected by `.` following the parsed type parameters. Qualified receivers (`fun models.User.greet()`) are rejected like every qualified type name today; when qualified type paths arrive, the last `.` segment is the name, as in Kotlin.

Rejected: accepting nullable receivers now (`fun String?.orEmpty()`), which needs its own rule for which of `T` and `T?` an extension call on a `T?` picks, and accepting generic built-in receivers before generics define how `T` binds. Both only accept more programs later.

### D4. `this` is its own expression node and a `val` binding (autonomous)

The parser produces `ExprKind::This` for the keyword. In the checker, `check_function_body` binds the receiver in the parameter scope, before the parameters, as a `val` variable under the reserved name `this` (no user name can collide, because `this` is a keyword), so defaults may read it, as in Kotlin. The place helpers (`is_place`, `infer_place`, `check_move_source`, the `&mut` binding check in `infer_unary_op`, `check_assignment`) treat `This` exactly like an identifier bound to that variable. `this` outside an extension is `E4001`, with a message saying that `this` is only available in an extension function.

`this` is a `val` in every mode, like every parameter: a `consuming fun` cannot assign `this.name` or call a `mut fun` on `this` without first moving it into a `var` (`var b = this`).

Rejected: parsing `this` as `Ident("this")`, which saves a few match arms but loses the distinction for the formatter and the LSP and would let a hand-built AST declare a local named `this`; a mutable `this` in `consuming fun` (Swift's consuming parameters are mutable), because parameters are immutable everywhere else and a `var` copy states the intent; and a new error code for `this` outside an extension, since it is an unresolved name.

### D5. Implicit `this`: locals, then receiver fields, then receiver extensions, then the module (user, extended autonomously)

In an extension body, a bare name `n` that is not a local or parameter resolves to `this.n` when the receiver's struct has a field `n` (user: "names resolve to `this.name` unless a local or parameter shadows them"). **(autonomous)** When `n` is called (`n(...)`) and no local, parameter or field matches, an extension named `n` on the receiver type resolves the call to `this.n(...)`. Only then are top-level declarations and imports searched. The implementation rewrites such a name into a synthesized `MemberAccess { target: This, member: n }` before checking it, so field reads, writes (`name = n`), borrows and moves through implicit `this` all go through the ordinary member rules, and the typed tree later sees the explicit form.

Rejected: fields only, which forces `this.greet()` for every call to a sibling method, unlike Kotlin and Swift; and module declarations before receiver members, which makes a struct field unreachable by its bare name whenever any top-level declaration shares it.

### D6. Members first, then the file's own extensions, then imports (user, ordering of built-ins autonomous)

A call `recv.name(args)` with a receiver that is not a type name resolves in this order:

1. A field `name` of the receiver's struct, looking through references, `Heap<T>` and, for `?.`, the nullable (`has_field`): the field is read and called, as today. A field takes precedence over an extension (from `value-semantics`).
2. **(autonomous)** The built-in `clone()` when `name` is `clone` and the receiver's type is `Clone`. Members, including built-in ones, win over extensions, as in Kotlin.
3. An extension whose receiver type equals the receiver's type after peeling references and `Heap<T>` (D8): the file's own extension if there is one (user: own-file shadowing), else the single matching import. Several imported matches report `E4012` naming their modules (user).
4. Otherwise `clone` falls back to `infer_clone_call` (`E4018`), and any other name to `member_type`, which reports `E4014` for a type whose module is not imported and `E4006` otherwise. `E4006` gains a help line when the receiver is nullable (D10).

Extensions live in their own table in the checker, keyed by receiver type and name, separate from the name scopes: an own-file table and an import table with one candidate per exporting module, mirroring `ScopeStack`'s import layer. So `fun greet()` and `fun User.greet()` coexist, extensions on different receivers never collide, and `greet` alone never names an extension outside implicit `this`. Ambiguity is per receiver type and name, so two imports exporting `describe` on different receivers are not ambiguous.

`recv.name` without a call is not a method value: when no field matches but an extension does, `E4006` gets help suggesting `recv.name()`. Method references are a non-goal.

### D7. Rejected extension declarations reuse `E4010` (ROADMAP for `clone`; field rule and code autonomous)

The checker rejects, with `DuplicateSymbol` (`E4010`):

- two extensions in one file with the same receiver type and name;
- **(autonomous)** an extension named like a field of its receiver struct, because the field always wins (D6);
- an extension named `clone` on a `Clone` type, including built-in ones (`String`, `Int`), because the built-in always wins (from `value-semantics`). On a type that is not `Clone`, such an extension is accepted and `t.clone()` calls it.

Rejecting the field clash at the declaration was preferred to accepting it silently (Kotlin warns, and Prism has no warnings channel): a struct that gains a field in another file would otherwise silently redirect every call of the extension to the field. With the error, the extension's file reports it when it is recompiled against the new ABI. When field visibility arrives, the rule applies to fields visible where the extension is declared. Reusing `E4010` follows Swift, which reports an extension member that clashes with a stored property as an invalid redeclaration; a new code was rejected because it is the same kind of error, two declarations with one name in one namespace.

### D8. Receiver passing: auto-deref, then the mode decides (user for implicit borrows and `E4003`; depth and `Heap<T>` autonomous)

At a call, the receiver expression is classified like an argument (D2 of `reference-access`): a place is inferred in place; any other expression is inferred in a consuming position and becomes a temporary that lives until the end of the statement. A `?.` chain of field accesses over a place counts as a place here (D10). Then a single function, `pass_receiver(found, extension, is_place, expr)`, peels every `&`, `&mut` and `Heap<T>` layer of the receiver's type until it equals the extension's receiver type **(autonomous: any depth, as Rust's auto-deref; `Heap<T>` included because field access already reads through it)**, and decides by mode:

- `fun`: a shared borrow of the place reached, of the temporary, or a reborrow through the peeled references. Never an error.
- `mut fun`: a mutable borrow. The place must be writable by the rules of `infer_place` and `contents_writable`: a `var` binding, a field path whose hops are all mutable, anything reached through `&mut` (so `m.rename()` for `val m: &mut User` reborrows, as passing `m` does), or a temporary **(autonomous, matching `&mut make()`, which is accepted today)**. Anything else is `E4003`, with help suggesting `var` for a `val` binding. This is the one exception to explicit `&mut` at call sites (user): `(&mut u).rename()` would be unreadable, and the method's declaration already says it mutates.
- `consuming fun`: a move. With no layers peeled, the receiver is a consuming position: `check_move_source` accepts a whole local, parameter or temporary and reports `E4017` for a field, top-level `val`, index or safe navigation. With layers peeled, a `Copy` receiver is copied out and any other is `E4017`, with the source "borrow `r`" (as in `reference-access` D4) or, through `Heap<T>`, the contents of the `Heap` **(autonomous: moving out of a `Heap<T>` waits for the `Heap<T>` change)**.

The result is a `ReceiverAdjustment { dereferences, pass }` with `pass` one of `SharedBorrow`, `MutableBorrow`, `Move` and `CopyOut`, plus whether the receiver is a temporary. Like `Adjustment`, it is returned but not stored until the typed tree exists. The typed tree (step 5) records it on the call, with the receiver normalized as the call's first argument, and MIR lowering (step 6) emits the borrow at the call, after the arguments are evaluated, which is what two-phase borrows (step 8, `v.push(v.size())`) need. The type checker does no conflict checking, so that call already typechecks.

Rejected: requiring the receiver's static type to match exactly (`(*r).greet()` does not even parse, since `*` is writes-only); one layer of auto-deref only, which would reject `rr.greet()` for `rr: &&User` although fields read through any depth.

### D9. Consuming receivers follow the existing move rules (user, from `value-semantics` E4017)

"Consuming Positions" gains the receiver of a `consuming fun` call, and nothing else changes: the receiver is checked exactly like a by-value argument. `u.toAccount()` moves `u` (use after move is `borrowck-moves`), `o.user.toAccount()` is a field move, `r.toAccount()` for `r: &User` a move out of a borrow, and `p.moved()` for `p: &Point` with `Point : Copy` a copy. One rule for every consuming site keeps `E4017`'s help consistent: borrow, clone, or take the whole value.

### D10. Safe-navigation calls, and `?.` chains over places as receivers (autonomous)

`recv?.name(args)` with `recv: T?` (or a borrowed nullable, through the view of `reference-access` D8) calls the extension on the payload and gives `null` otherwise; the result type is the return type made nullable, never nested (`Int?` stays `Int?`, `Unit` becomes `Unit?`). The arguments are evaluated only when the receiver is not `null` (planned runtime meaning, `docs/architecture.md` §4). `?.` on a non-nullable receiver behaves like `.`, as it does for fields. A `.` call on a nullable receiver is `E4006`, with help suggesting `recv?.name()`.

A receiver that is a chain of `?.` field accesses over a place (`t?.lead` in `t?.lead?.greet()`) is treated as an optional place: when every link is present it is borrowed, or mutably borrowed, where it lives. Under the plain rule of `reference-access` D2, a safe navigation is a temporary, so `t?.lead?.greet()` would move `lead` (`E4017` for a non-`Copy` field) and reject the most common Kotlin method-call idiom. Optional places are safe for receivers because the call borrows the payload `&User`, never a `User?`: the typing problem that keeps `&u?.nick` a temporary (there is no stored `String?` for a `&(String?)` to point at) does not arise. A `consuming` call on such a chain still moves a field and is `E4017`. Extending optional places to explicit `&` and to `(&T)?` arguments stays the existing ROADMAP follow-up.

Rejected: the plain temporary rule (consistent with `&u?.nick`, but rejects `user?.address?.city?.shout()`); and nullable receiver extensions as the way to call on a `T?` (D3).

### D11. Visibility and imports (user for import visibility; receiver-module rule autonomous)

An extension is visible when declared in the file or exported by a direct import (`public`, or `package` within the same package, `E4007` otherwise), as for functions; private extensions are not exported. **(autonomous)** Calling an imported extension does not need the receiver type's module imported: the call is a function of the extension's module, whose ABI already binds the receiver type, and the receiver value can be received and passed along without `E4014` today. Declaring an extension names its receiver type in an annotation, so the existing annotation rules apply there (`E4007`, `E4012`, `E4014`). The body of an extension reads fields only through its own file's imports. Builds stay hermetic: lookups read only the direct imports' ABIs.

Rejected: requiring the receiver's module too, which would make `first().shout()` need an import that the caller never uses by name.

### D12. ABI: a receiver on the function record (autonomous representation; struct-hash rule from the exploration)

`ExportedSymbol::Function` gains `receiver: Option<AbiReceiver { mode: ReceiverMode, ty: AbiType }>`. Phase 1 lowers the receiver type with the same binder as parameter types, so it is bound to its defining module and carries its marker. `calculate_hash` writes a presence byte, the mode and the type. `sort_key` adds the receiver's module-qualified type text after the name (functions without a receiver first), so `describe`, `User.describe` and `Team.describe` hash the same in any order. Struct and enum records are untouched, so declaring an extension never changes the receiver type's record, and an extension in another file never changes the receiver type's file hash (exploration: "a struct's ABI is only its layout and `Copy`ness"). An exported extension in the type's own file changes that file's hash, like any new export. A non-generic extension's body is not hashed. The ABI needs nothing else for elision rule 2, which reads the receiver mode.

The checker's `abi_symbol_types` includes the receiver type, so `record_nominals` learns its marker, and `ingest_file_abi` puts extensions into the import extension table (D6) instead of the name scopes.

Rejected: a separate `ExportedSymbol::Extension` variant, which would duplicate the parameter, default and generic-body handling; and attaching extensions to their receiver's record, which would make every extension change the receiver's ABI.

### D13. Inference of unannotated private extensions

Private extensions may omit the return type like private functions, and are inferred on demand. `inference`, `recursive_inference` and `ensure_inferred` are keyed by name today, which would conflate `User.size` and `Team.size`. They become keyed by an `InferenceKey` (`Declaration(name)` or `Extension { receiver, name }`), and call resolution calls `ensure_inferred` for the extension it picked.

### D14. No new error codes (autonomous)

- `E4001`: `this` outside an extension (D4).
- `E4003`: a `mut fun` receiver that is not a mutable place, and writes through `this` in a plain `fun` (D8).
- `E4006`: no field or visible extension, a `.` call on a nullable receiver, or an extension read without a call, each with help (D6, D10). `TypeError::UnknownField` gains an optional `help`.
- `E4010`: duplicate extensions, an extension named like a field of its receiver, and `clone` on a `Clone` type (D7).
- `E4012`: an extension exported by several imports (D6).
- `E4017`: a `consuming` receiver that cannot be moved (D9).

Each is the same kind of error its code already names, and the catalog rows are widened. This follows the precedent of `E4017` gaining the borrow source in `reference-access`.

### D15. A new `extension-functions` capability (autonomous)

Declaration, resolution, implicit `this`, declaration checks and safe-navigation calls are about one subject and will grow with traits, static functions and extension properties, so they get their own spec, as `reference-access` created `references`. The modes' call-site effects stay with the specs that own those rules: borrows in `references`, moves in `ownership`, grammar in `syntax` and the ABI in `compilation-and-abi`. Rejected: putting it all in `type-system`, already the largest spec.

## Risks / Trade-offs

- [Implicit `this` lets a receiver field or method hide a top-level declaration of the same name inside extension bodies] → this is Kotlin's and Swift's rule. A top-level declaration hidden this way is still reachable outside extension bodies, and qualified paths, when they exist, will reach it inside.
- [Rejecting an extension that a field shadows means adding a field can break another package's extension file] → that file is recompiled anyway, because the type's ABI changed, and the error names the field; the alternative is a silent change of what every call does.
- [Optional places apply to `?.` receivers but not yet to `&u?.nick` or `(&T)?` arguments] → the follow-up already on `ROADMAP.md` extends them; until then `&u?.nick` keeps its `E4017`, and the specs say which form applies where.
- [Any-depth auto-deref through `Heap<T>` lets a `consuming` call look like it could move out of a `Heap`] → it is `E4017` until the `Heap<T>` change decides whether contents can be moved out.
- [`mut fun` on a temporary is accepted, though the mutation is lost when the temporary is dropped] → it matches `&mut make()` today and Rust's `String::new().push('a')`; it is useful once mutating methods return values (builders).
- [One extension per receiver and name] → overloading by parameter types is not part of Prism's function model either; default and named parameters cover most uses.

## Migration Plan

No external users. In-repo, `examples/demo/models` turns `updateName` and `rename` into `mut fun User.rename(...)` once the feature works; existing free functions stay valid. No existing fixture changes its expectation: `this`, `consuming fun` and `fun T.name` were all syntax errors, and `x.f()` on a field keeps its meaning because fields win. Rollback is a revert.

## Open Questions

- The exact wording of the new `E2001` messages and help lines can be tuned during implementation.
