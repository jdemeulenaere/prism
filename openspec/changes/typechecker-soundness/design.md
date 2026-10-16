# Design: typechecker-soundness

## Context

See proposal.md (Why) for motivation. These facts about the code at `8fb2a5e` (after the plain-fix commits) shape the approach:

- **`Type` carries members.** `Type::Struct { name, fields }` and `Type::Enum { name, variants }` hold their member lists. An annotation lowers to a field-less `Struct { name }` (`Type::from_ast`), and `resolve_nominal` patches that up lazily at read sites.
  - Nominal equality in `is_assignable_to` is by bare name, and it even lets an `Enum` equal a `Struct` so annotations can stand in for enums.
  - The plain fix `e113b9b` made `&T` compare its inner types by assignability, so `&User` now matches `&User`, but equality is still name-only.
- **Exported types are strings.** `ExportedSymbol` stores types as the AST's `Display` string, and the importer re-parses them with `Type::parse_str`.
  - That parser splits on the first `) -> ` and on every `,`, so `((Int) -> Int) -> Int` breaks, and `(Int) -> Int?` comes back as `((Int) -> Int)?`.
  - It also resolves every name in the *importer's* scope, which is why `o.user.id` picks up the wrong `User` (repro 3a).
- **Two passes, file order.** `check_module` runs Pass 1 (registration) and then checks bodies in file order. An unannotated private declaration stays `Type::Error` until its own body is checked, so earlier uses, and recursive uses, go unchecked.
- **Imports share the module scope.** `ingest_file_abi` defines imported symbols with `define(..).ok()`, so when two imports export the same name, the first one silently wins.
- **`check_expr` reports every leaf mismatch as `E4002`.** `E4005` appears only for a bare `return` and a top-level `return`. A `Unit` block body passes `Some(Unit)` as the expected type of its final expression.
- **Closures in inference mode** bind every parameter to `Type::Error`.
- **Constraints:**
  - The invariants (single-file compilation units; deterministic hermetic builds and compilation avoidance), and Phase 1 must not read bodies.
  - `harden-parser-grammar` changes the closure AST node and function-type parsing.
  - A plain-fix session is editing the same checker files concurrently.

## Goals / Non-Goals

**Goals:**
- Nominal type identity the borrow checker can rely on, in one file and across files.
- A clear, specified rule for reading through references, error codes for each kind of mismatch site, and inference rules that do not depend on declaration order.
- Every behavior change lands with a regression test that runs real `.prism` source.

**Non-Goals (design-level, in addition to proposal.md):**
- No typed IR or side tables keyed by node.
- No `Copy` or move classification. D1 avoids needing one.
- No warnings channel.

## Decisions

### D1. Reading through references: auto-deref in read positions (finding 6). **Confirmed by the user on 2026-10-16.**

**Choice:** read positions see through references.
- **Read positions:** operands of arithmetic, comparison, equality, `&&`/`||`, unary `-`/`!` and `..` bounds, and `if` and subjectless-`when` conditions.
- **How:** every `&` and `&mut` layer is peeled before the operand is typed, and the result of an operator is a fresh value.
- **Already consistent:** indexing and field access already peel references.
- **`when` subjects** follow the same rule. Peeling them is part of the plain fix for finding 19, which should use this rule.
- **`Heap<T>`** is not peeled by operators.

**Not read positions:** where a `T` is *expected* (arguments, initializers, returns, assignment values), a `&T` is still rejected. Copying a value out of a borrow is the borrow checker's decision, once it knows which types are `Copy`. Allowing it now would silently admit moves out of borrows for non-primitive types.

**Alternative: an explicit prefix `*` (`*a + 1`).** This is closer to Rust and makes every read visible. It was rejected because:
1. It needs a lexer and parser change, and the parser is being changed in parallel by `harden-parser-grammar`.
2. Field access already reads through references implicitly (`user.name` with `user: &User`), so explicit `*` for operators only would be inconsistent.
3. It does not settle *writing* through `&mut Int` either: `*r = 5` also needs a new place form.

Writing through a `&mut` primitive stays impossible after this change and is a follow-up.

### D2. Return and argument mismatches use their dedicated codes (finding 7). **Confirmed by the user on 2026-10-16.**

**Choice:** align the code with the current spec and catalog. A value checked against the return type of its enclosing function or closure reports an incompatible type as `E4005`. A value checked against a parameter type reports it as `E4004`. Every other expectation (annotation, assignment, operator operand, default value) keeps `E4002`.

**Mechanism:** the checker tracks the *origin* of the current expectation (`Value | Return | Argument`) in a field that is saved and restored around each site. Leaf assignability mismatches pick their code from the origin:
- **Sets `Return`:**
  - a `return e` operand;
  - the expression body of a function with a declared return type;
  - the final expression of a block body whose declared return type is not `Unit` (a `Unit` body discards it, D8);
  - a closure body checked against an expected return type.
- **Sets `Argument`:** every call argument (positional, named, trailing lambda), struct constructor arguments and enum payload arguments.
- **Inherited** through `if`/`when` branches and a nested block's final expression. Example: in `return if (c) "a" else 1`, the mismatch on `"a"` is `E4005`.
- **Reset to `Value`** by every construct with its own expectation: local initializers, assignments, operator operands, and nested calls, whose arguments get `Argument` for that call.

A new `TypeError::ArgumentTypeMismatch { expected, found, span }` maps to `E4004`, alongside `CallSignatureMismatch`. One code may have several variants, as `E3001` already does. Non-assignability errors keep their codes. In particular, `TypeNameAsValue` stays `E4002` even in a return position (`fun f(): User = User`), because it is not a mismatch between two types.

**Alternative: narrow the spec instead.** Keep `E4002` for every value mismatch, and keep `E4004`/`E4005` only for shape errors (arity, names, a bare `return`). This is less code and close to Rust's single `E0308`, and it avoids the ripple below. It was rejected because the catalog already promises the dedicated codes, tools key on codes, and site-specific codes let a future LSP offer site-specific fixes ("change the return type" versus "change the argument"). If the user picks it, the type-system requirement "Return and Argument Mismatch Codes" is replaced by a MODIFIED version of the existing scenarios "Arity or argument type mismatch rejected" and "Return statement type verification". Tasks 3.x then shrink to that spec edit and the catalog edit.

**Ripple of the chosen option:**
- **`regressions.rs`:** `mutable_reference_stays_invariant` (`take(&mut x)`, an argument) becomes `E4004`. `branches_join_with_null_into_nullable_type` (`return x`) becomes `E4005`.
- **Unit tests in `checker.rs`** that expect `E4002` at these sites change the same way.
- **`harden-parser-grammar`'s scenarios** for `takesInt { 42 }` and `later(content)` (arguments) become `E4004`. Once that change is archived, this change adds MODIFIED blocks for its requirements "Parameterless Closures Are Checked as Closures" and "Composable Function Types Keep Their Compose Flag" (task 1.2). They cannot be written yet, because those requirements are not in `openspec/specs/`.

### D3. Nominal identity: `Type` holds an ID, and definitions live in a type table (finding 3)

`Type::Struct(NominalId)` and `Type::Enum(NominalId)` carry no members. `NominalId { module: ModulePath, name: String }`, where `ModulePath` is the root-relative file path without its extension (`a/lib`), which is exactly the root-anchored import path. Definitions (fields with `var` flags, variants with payloads) live in a `TypeTable: HashMap<NominalId, TypeDef>` owned by the checker, filled from the file's own declarations and its direct imports' ABIs.

- **Equality is ID equality.** A struct never equals an enum. `Type` stays cheap to clone and hash. The plain fix's covariant `&T` rule keeps working on top of it.
- **Annotations bind eagerly.** Lowering an annotation resolves the name in the type namespace to a `NominalId` at the point where the annotation appears. A name with no declaration becomes `Type::Opaque(name)`. That keeps "unknown names stay opaque", and opaque types compare by name.
- **The built-in `Range` is `NominalId { module: "<builtin>", name: "Range" }`.**
- **The checker learns its module path.** `TypeChecker::new` takes the module path in addition to the package. The CLI derives it from the file's path relative to the workspace root. The test helpers in `regressions.rs` take a module path for each source, and a single-file helper defaults to `"<test>"`.
- **Supersedes the "Nominal types resolve lazily" rationale** in the type-system spec. Lazy resolution was right while types carried field lists, because eager resolution then meant cycle guards and whole-list equality. With IDs, the name binds eagerly and the definition is looked up lazily in the table, so neither cost exists. `resolve_nominal` is removed.
- **Interim ingestion, until D4 lands:** an ABI type string's name is resolved first against the *exporter's* own exported types (exporter module), then against the importer's scope (today's behavior). That already fixes repro 3a. D4 removes the fallback.

**Alternative: keep member-carrying types and add a module field to `name`.** That fixes cross-file conflation, but it keeps the annotation-versus-constructor asymmetry and makes equality compare field lists. Rejected for the reasons in the superseded rationale.

### D4. Structured, module-qualified ABI types (finding 20)

`prism-abi` gains `AbiType`:
- `Prim(PrimKind)`
- `Unit`
- `Nominal { module: Option<String>, name, args: Vec<AbiType> }`
- `Nullable(Box<AbiType>)`
- `Ref { mutable, inner }`
- `Heap(Box<AbiType>)`
- `Fn { compose, params, ret }`

`ExportedSymbol` stores `AbiType` instead of `String`, and `FileAbi` gains `module: String`. The hash walks the tree with one tag byte per node kind, so structure is hashed and never re-parsed. `Type::parse_str` is deleted, and the checker converts `AbiType` to `Type` directly.

**Binding nominal names without reading bodies.** Phase 1 is split into two steps:
- **1a** is today's syntactic surface extraction, per file, with no dependencies.
- **1b** binds each nominal name in an exported signature to a module:
  1. the file's own top-level type declarations, including private ones;
  2. otherwise, the type names exported by its direct imports' 1a surfaces.

  A name found in two imports, or in none, stays `module: None`. The ambiguous case is reported by Phase 2 as `E4012` when it checks the same declaration (D7). An unresolved name stays opaque.

Step 1b reads only direct imports' 1a outputs. It works with import cycles and never touches a body. `FileAbi::extract` takes the module path and a resolver closure (`&dyn Fn(&str) -> TypeBinding`), so `prism-abi` stays independent of `prism-resolver`. At archive, the compilation-and-abi rationale "The ABI comes from the AST" is amended to "the AST and the direct imports' exported type names, never bodies".

**Alternative: keep strings but fix the parser.** A balanced-parenthesis parser for `Type::parse_str` would fix the higher-order cases. Rejected:
- the type text would still be parsed twice, by two different parsers that can drift apart;
- the name binding would still happen in the importer's scope (repro 3a);
- the hash would still cover formatting choices of `Display`.

**Finding 21** (a type reachable only through an import) is decided in D9.

### D5. Order-independent inference (finding 12)

**Choice:** on-demand inference with cycle detection.
- **Inference states.** Unannotated private top-level declarations get `Pending | InProgress | Done(Type)`.
- **On-demand check.** When an identifier resolves to a `Pending` symbol, its body or initializer is checked immediately, in a **fresh body context**: the module scope only, with no enclosing locals, the declaration's own compose flag, its own expected return and a `Value` origin.
- **Memoization.** The type is memoized, and the main pass skips bodies already `Done`, so no error is reported twice.
- **Cycles.** Reaching an `InProgress` symbol means the inference depends on itself (`fun fact(n: Int) = … fact(n - 1)`). That reports `E4013` ("add an explicit return type") and yields `Error`.

The fresh body context is a small struct: the local scope stack, the return expectation, the compose stack, `in_function` and the mismatch origin. It is swapped out and back, and the module scope is shared.

**Alternatives:**
- **Require annotations on every top-level declaration.** This is simplest, but it removes a convenience the spec grants to private expression-bodied functions.
- **A pre-pass in dependency order.** This is equivalent to on-demand inference, but it needs a separate free-variable walk over every body. On-demand reuses the checker itself.

Kotlin also rejects recursive functions without an explicit type, so the `E4013` case matches what Kotlin users expect.

### D6. Closure parameters without an expected type (finding 11, first part)

**Choice:** in inference mode with no expected function type, each closure parameter without a type annotation reports `E4013` ("cannot infer the type of closure parameter `x`") and is bound to `Error`, so the closure body is still walked for other errors.
- **Parameterless closures** (after `harden-parser-grammar`, `val f = { 1 }`) have nothing to infer and type as `() -> T`.
- **Parameter annotations.** `harden-parser-grammar` leaves typed closure parameters to a later change, so today every parameter in inference mode reports `E4013`. The requirement is worded "expected type or annotation", so a later grammar change only adds the annotation path.

**Alternatives:**
- **Infer from first use** (`g("str")` makes `x: String`). This needs constraint solving across statements, which the bidirectional design deliberately avoids.
- **Keep `Error`.** This is unsound: the body and every call go unchecked.

### D7. Imports form their own layer; ambiguity is reported at use (finding 15)

Imported symbols go into an import layer *below* the module scope. Each name maps to the set of candidate symbols, de-duplicated by defining module.
- **One candidate:** the symbol is used.
- **Several candidates:** `E4012` (new), listing the modules, reported at the use site (identifier, callee or annotation type name). The result is `Error`.
- **A top-level declaration with the same name as an import** still reports `E4010`. That is today's behavior, pinned by a test. Letting local declarations silently shadow imports is a separate language decision, listed as a follow-up.

**Alternatives:**
- **Report at the import.** This is stricter, but it makes two libraries that both export `helper` impossible to import together, even when the file never uses `helper`. Reporting at use matches Kotlin and Go.
- **Last import wins, or first import wins (today).** Silent, order-dependent and confusing. Rejected.

### D8. Discarded block values (finding 16)

`check_block` takes a `discard: bool` alongside its expected type. The value of a block body is **discarded** when:
- it is the body of a block-bodied function returning `Unit`, declared or implied;
- it is the body of a closure whose expected return type is `Unit`.

A discarded final expression is *inferred*, so its errors are still reported, and its type is ignored. An expression body (`fun f(): Unit = g()`) is not discarded and is still checked against `Unit`, because the author wrote the value explicitly. That case reports `E4005` under D2.

**Alternatives:**
- **Require the final expression to be `Unit`** (today). It rejects `fun f() { list.add(x) }`-style code whenever the last call returns a value, which surprises Kotlin users.
- **Discard in expression bodies too.** It hides real mistakes in `fun f(): Unit = compute()`.

The plain fix for finding 19a (a statement-position `when` without `else`) needs a notion of "value discarded" as well. Whichever lands first defines the flag, and the other reuses it.

### D9. Members of a type require importing its defining module (finding 21). **Decided by the user on 2026-10-16.**

**Choice:** a file may receive and pass along a value whose nominal type is defined in a module it does not import directly. For example, `first()` comes from a direct import and returns `Order` from `shop/models`. But reading the value's members (field access, field assignment, matching on its enum variants, constructing it) requires a direct import of the defining module. Without one, the checker reports a new error, `E4014`, naming the type, its defining module and the import to add (`add 'import /shop/models' to use the members of 'Order'`). The result is `Error`.

- **Why it stays hermetic:** the checker reads only the ABIs of direct imports. After D3 and D4, `Order` arrives as a `NominalId` with its defining module, so the checker can name the module without loading it.
- **Assignability is unaffected.** It compares IDs, so passing the value to a parameter of the same type, from a direct import, works.

**Alternatives rejected by the user:**
- **Embed the shape of each mentioned type in the exporter's ABI.** Hermetic and import-free for callers, but it duplicates type definitions across ABIs.
- **Relax the hermetic-builds invariant** to allow reading transitive ABIs.

## Risks / Trade-offs

- **[Risk] The D3 `Type` refactor touches most of `checker.rs`** while a plain-fix session edits the same file. → **Mitigation:** apply this change only after the plain-fix work on the checker is committed, and land D3 first as its own commit with no behavior change beyond identity. Hand-built unit tests that exercise behavior move to `regressions.rs`.
- **[Risk] Programs accepted today are rejected** (name-only matches across modules, recursive inferred functions, unannotated closure parameters without context). → **Mitigation:** each is listed as BREAKING in proposal.md. The examples are re-checked by the apply guidance.
- **[Risk] Every ABI hash changes once** (D4). → **Mitigation:** there is no `.abi` cache yet. The hash tests in `prism-abi` assert relative properties (changed versus unchanged), which survive.
- **[Trade-off] D1 keeps value-for-reference coercion strict.** `g(a)` for `g(x: Int)` with `a: &Int` still errors until the borrow checker defines `Copy`.
- **[Trade-off] D2's origin tracking is ambient state.** A missed save/restore yields the wrong code, but never a wrong accept or reject. → **Mitigation:** tests pin the code for each site (`return`, expression body, block tail, closure body, positional, named and trailing-lambda arguments, constructor field).

## Migration Plan

1. **Land the plain-fix commits** on `crates/prism-typechecker` and **apply `harden-parser-grammar`**, which changes `ExprKind::Closure` and `TypeKind::Function { is_compose }`.
2. **Rebase this change onto both.** Then:
   - re-verify the repros;
   - port `TypeKind::Function { is_compose }` into `AbiType::Fn { compose }` (D4);
   - add the MODIFIED blocks described under D2.
3. **Land the task groups in order.** Identity (D3) first, because the ABI work (D4) builds on `NominalId`.
4. **Rollback is per task group**, by reverting its commit. Only D3 and D4 change data structures that later groups depend on.

## Open Questions

- **How `E4012` should list candidate modules** (import path, or package plus file). Wording only; it does not affect codes or behavior.
