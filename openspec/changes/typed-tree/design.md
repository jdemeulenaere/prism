# Design: typed-tree

## Context

See `proposal.md` for motivation. This change is step 5 of "the road to the borrow checker" in `ROADMAP.md`, agreed with the user in the `/opsx:explore` session of 2026-10-17: a `prism-typed-tree` crate produced by the type checker, with resolved bindings, a type on every node, normalized calls and explicit copy, move and borrow adjustments, including the receiver adjustments of `extension-methods`. The planned requirement is `docs/architecture.md` §1, "Semantic Pipeline After Type Checking", scenario "Typed tree from the type checker". The user was not available while this change was proposed, so every question beyond that is decided here, marked **(autonomous)**, and logged in `docs/decisions-for-review.md`.

The tree is the input of `mir-lowering` (step 6, a place-based CFG), and later of `generics` (step 10, monomorphization), the Cranelift backend (steps 14 to 16), `closures` (step 17, closure conversion), `hot-reload` (step 18) and `compose-transform` (step 20). Each decision below says which of them it serves.

Current state:

- **Checker** (`crates/prism-typechecker/src/checker.rs`, about 4000 lines plus tests). `check_module` registers declarations, then checks bodies, and leaves its diagnostics in `errors`; it returns nothing. Every `infer_*`/`check_*` function returns a `Type`. Two flags carry the ownership decisions: `consuming` (set before checking a value in a consuming position) and `parent_consuming` (inherited by branches, block tails and `?:` operands); `check_move_source` reports `E4017` for consumed leaves. `coerce` classifies how a value fits its position as an `Adjustment` (`None`, `CopyOut { layers, through_nullable }`, `Borrow`, `BorrowTemporary`, `BorrowContents`, `Reborrow`, `NullableView`), and `fit` applies it, deferring a leaf's move check until the adjustment is known. `pass_receiver` classifies a receiver as a `ReceiverAdjustment { dereferences, through_nullable, pass, temporary }`. Both are `#[allow(dead_code)]`: computed, then dropped. Implicit `this` is a rewrite into a synthesized `this.name` AST node (`implicit_this_field`, `implicit_this_callee`). Unannotated private declarations are inferred on demand (`infer_declaration`), which checks another body in the middle of the current one, saving and restoring a `BodyContext`.
- **Types** (`crates/prism-typechecker/src/types.rs`). `Type`, `NominalId`, `TypeDef` and `BUILTIN_MODULE`, plus `Type::from_ast`, `Type::from_abi`, `is_assignable_to`, `join` and the renderers.
- **Scopes** (`scope.rs`). `SymbolKind::Variable` and `SymbolKind::Function` record type, mutability or parameter names, visibility and package, but neither a binding identity nor the defining module. Imported symbols live in an import layer keyed by module, extensions in the checker's own tables. Payload-free enum variants are bound as `val` symbols and payload variants as function symbols, told apart by `variant_owners`.
- **AST** (`crates/prism-ast`). Boxed, with spans and no node ids. `Expr`, `Block` (statements plus an optional tail) and `Statement` (`Local`, `Assign`, `Expression`, `While`, `DoWhile`, `For`).
- **CLI** (`crates/prism-cli`). `validate_and_extract_abi` in `workspace.rs` runs Phase 1 over the whole graph and then Phase 2 in topological order, stopping at the first file with errors. `main.rs` has `check`, `dump-ast` (Rust `Debug` output), `abi` and `tokens`. Fixtures (`tests/fixture_tests.rs`) compare only the set of reported codes.
- **A latent double check.** `check_expr_kind` checks an `if` against an expected type; with no `else` and a non-`Unit` expectation it falls through and infers the whole `if` again, so `val x: Int = if (c) nope` reports `E4001` twice today.

## Goals / Non-Goals

**Goals:**
- Every decision the checker makes about a body (resolution, types, coercions, moves, copies, borrows, call shapes, receiver passing, loop targets) is recorded exactly where it is made, so no later stage re-derives one.
- The tree has what MIR lowering needs to produce places, operands, temporaries and branches without consulting the checker's rules, and what the later stages need to find composable calls, captures and declaration identities.
- A tree is produced for files with type errors, and trees of error-free files are verifiably free of error nodes.
- No behaviour change: every existing fixture keeps its expectation. The only observable differences are the new command and one fewer duplicated diagnostic.
- Every scenario has a fixture with a snapshot of its tree.

**Non-Goals:**
- MIR, drop elaboration, temporary lifetimes beyond "until the end of the enclosing statement", and how default values are evaluated at run time (step 6 and the backend).
- How closures capture (by shared borrow, mutable borrow or move): `borrowck-exclusivity` infers it from the recorded captures.
- Type arguments on calls and generic instantiation (`generics` adds them).
- Interning types, caching or serializing trees, incremental re-checking.
- Parser error recovery: a file that does not parse has no AST and no tree.

## Decisions

Decisions marked **(user)** were agreed in the exploration session; **(autonomous)** ones were made while proposing this change and are open to review. The others follow from these or from existing rules.

### D1. The checker builds the tree while checking (user for "produced by the type checker"; direct building autonomous)

Every `infer_*`/`check_*` function in `checker.rs` builds the node for the expression it checks and returns its `ExpressionId`; the node's type is read back from the body being built. The point that decides something records it: `fit` wraps the node it fits, `pass_receiver` builds the receiver argument, `check_call_arguments` returns the normalized arguments, `infer_expr` wraps a consumed place. Implicit `this` is already a rewrite into an explicit `this.name` AST node, so the tree sees the explicit form for free.

On-demand inference checks another declaration in the middle of a body. The body builder is part of `BodyContext`, so `infer_declaration` saves the current builder, builds the other declaration's body to completion, and restores it, exactly as it already does for locals and flags. Every body is still built once.

The one place that checks an expression twice, the `if` without `else` in `check_expr_kind` (see Context), is changed to build the `if` once from its checked branches and report the missing `else` value directly as the mismatch it reports today (`E4002`/`E4004`/`E4005` by origin, "expected `T`, found `Unit`"). That removes the duplicated diagnostic, a plain fix with a regression test.

Rejected:
- **A second pass from side tables**, as Rust builds THIR from HIR plus `TypeckResults`. The AST has no node ids, so the tables would be keyed by spans (not unique: implicit-`this` rewrites and adjustments share spans) or require ids in the parser. The second pass would also re-derive the place classification, implicit `this`, argument matching and receiver lookup from the tables, which is the drift this change exists to remove.
- **Annotating the AST in place**, as Swift's type checker rewrites its AST. The AST is shared with Phase 1 (default values and generic bodies are hashed from it) and will be the formatter's input; semantic slots and synthesized adjustment nodes do not belong there.

Staging: the refactor touches every checking function, but it is mechanical once the node model exists, and a half-built tree has no consumer. It lands as one change, staged in `tasks.md` so that the tests pass after every group: first the crate, the model, the dump and the scaffolding with placeholder nodes, then one group of node kinds at a time, each verified by its snapshot fixtures, and last the validator that forbids placeholders in error-free trees.

### D2. `prism-typed-tree` owns the semantic `Type` (autonomous)

`Type`, `NominalId`, `TypeDef`, `BUILTIN_MODULE`, `Type::nullable`, `bind_names`, `qualified`, `render_pair` and `Display` move from `prism-typechecker/src/types.rs` to `prism-typed-tree/src/types.rs`. The checker keeps the rules: `Type::from_ast` and `Type::from_abi` become free functions (`type_from_ast`, `type_from_abi`) in the checker's `types.rs`, and `is_assignable_to`, `is_shared_view_of` and `join` become a `TypeRelations` trait implemented for `Type` there, so call sites keep their method syntax. `prism-typechecker` re-exports the moved items, so `prism_typechecker::Type` keeps working. `prism-typed-tree` depends only on `prism-ast` (spans, literals, visibility, receiver modes, markers).

Rejected: the tree crate depending on the checker for `Type`, which makes D1 a dependency cycle; a separate `prism-types` crate, since no consumer needs types without the tree and splitting it out later is a move of one file; interned type ids, premature while nothing is generic. Nodes store `Type` by value behind an accessor (`Body::type_of(id)`), so interning can come later without touching consumers.

### D3. Per-body arenas with typed ids (autonomous)

```rust
pub struct TypedFile { pub module: String, pub package: Option<String>, pub items: Vec<Item>, pub bodies: Arena<BodyId, Body> }
pub enum Item { Function(FunctionItem), Struct(StructItem), Enum(EnumItem), Value(ValueItem) }
pub struct Body {
    pub owner: DeclarationPath,
    pub locals: Arena<LocalId, Local>,
    pub expressions: Arena<ExpressionId, Expression>,
    pub statements: Arena<StatementId, Statement>,
    pub parameters: Vec<LocalId>,               // `this` first for an extension
    pub defaults: Vec<Option<ExpressionId>>,    // one per declared parameter
    pub value: Option<ExpressionId>,            // `None` for a function without a body
}
pub struct Expression { pub kind: ExpressionKind, pub ty: Type, pub span: Span }
```

`Arena<I, T>` is a vector indexed by a `u32` newtype (`ExpressionId`, `StatementId`, `LocalId`, `BodyId`). Items are in source order and point at their bodies. Ids are dense and allocated in checking order, so the tree of a file is deterministic.

A parameter's default value is an expression of its function's body (`Body::defaults`), checked where it is checked today, after the receiver and the earlier parameters are bound, so it reads them as the body's bindings. A default is evaluated at the call, so MIR lowering can lower each default as its own small function over those bindings (Swift's default argument generators), which keeps a caller from ever needing the callee's private declarations.

Why arenas: borrowck diagnostics, MIR source maps, the LSP's hover and the compose transform's group keys all need to name a node; `fit` wraps a node after it is built, which is an id swap; a body is the unit of MIR lowering, of parallel Phase 2 and of hot reload. Rejected: a boxed tree, whose nodes have no identity for side tables and which must be rebuilt to insert a wrapper; one arena per file, which mixes bodies that are lowered, inferred on demand and reloaded separately.

### D4. Bindings and declaration paths (autonomous)

- **Locals.** Every parameter, local, `for` variable, closure parameter and `this` is a `Local { name, ty, mutable, kind, span }` of its body, `kind` one of `Receiver`, `Parameter`, `Let`, `ForVariable`, `ClosureParameter`. `SymbolKind::Variable` gains `local: Option<LocalId>` (`None` for top-level values), set when the checker defines the binding, so resolving a name yields its `LocalId`. Shadowing yields distinct ids.
- **Declarations.** `DeclarationPath { module, receiver: Option<Type>, name }`, rendered `models/user::greet` or `models/user::User.greet`. It is the ABI's identity for an exported symbol (module path, receiver, name), which is unique because Prism has no overloading and one extension per receiver and name. Own declarations use the file's module path. Function and top-level value symbols, and `Extension`, gain the defining `module`, filled from `abi.module` in `ingest_file_abi` and from `module_path` in `function_symbol`.
- **Fields and variants** are referred to by index in their declaration, plus their name for dumps. The ABI records both in declaration order, so an index is as stable as the importer's view of the type, and code generation needs it for layouts and discriminants.

Rejected: global numeric definition ids, which need a workspace-wide table and break single-file compilation units (an index in another file shifts when that file reorders declarations, which the ABI hash deliberately ignores); names only, which conflate shadowed locals. Symbolic paths also serve the later stages: `hot-reload` keys its function-pointer table by them, and `generics` can serialize an exported generic body's tree into the artifact cache, because nothing in it points into another file's memory.

### D5. Places, and explicit moves, copies and borrows (user for explicit adjustments; node shapes autonomous)

Place nodes: `Local(LocalId)`, `TopLevelValue(DeclarationPath)`, `Field { base, index, name }`, `Deref { pointer }` (through a `&`, `&mut` or `Heap<T>`), `Index { base, index }`, `NonNullPayload { nullable }` (D7) and `Temporary { value }` (a value evaluated into storage that lives until the end of the enclosing statement). Value nodes for uses of places: `Move { place }` and `Copy { place }`. Borrows: `Borrow { mutable, origin, place }` with `origin` one of `Written`, `Argument`, `Receiver` and `Conversion`.

The rules:

- A place in a consuming position is wrapped in `Move`, or in `Copy` when its type is `Copy`. This happens where `check_move_source` runs today: in `infer_expr` for a consumed leaf, and in `fit` for a deferred leaf whose adjustment is `None`. A consumed value that is not a place (a call, a literal, a construction) is not wrapped: it is a fresh value.
- A place anywhere else appears unwrapped and is read in place: an operand or condition, a `when` subject, the operand of `&`, a receiver or callee read in place, a discarded statement, an assignment target. MIR lowering picks the operand form from the type.
- Every looked-through layer is an explicit `Deref`: field access (`member_type` peels `&`, `&mut` and `Heap<T>`), operands and conditions (`infer_operand` peels references), `when` subjects and receivers. MIR places are a local plus projections, so a field read through `&&User` is `Field(Deref(Deref(Local)))`.

How each existing classification becomes nodes, in one function per enum (`apply_adjustment`, `apply_receiver_adjustment`):

| Checker classification | Typed tree |
|---|---|
| `Adjustment::None`, consumed place | `Move(p)` or `Copy(p)` |
| `Adjustment::CopyOut { layers: n, through_nullable: false }` | `Copy(Deref^n(e))` |
| `Adjustment::CopyOut { layers: n, through_nullable: true }` | chain (`CopyOut`) of `Copy(Deref^n(NonNullPayload(e)))` |
| `Adjustment::Borrow` | `Borrow(shared, Argument, p)`, coerced to `(&T)?` when the parameter is optional |
| `Adjustment::BorrowTemporary` | `Borrow(shared, Argument, Temporary(e))`, or a `BorrowContents` chain over the temporary when `e` is a `T?` |
| `Adjustment::BorrowContents` | chain (`BorrowContents`) of `Borrow(shared, Argument, NonNullPayload(p))` |
| `Adjustment::Reborrow` | `Borrow(mutable as the parameter, Argument, Deref(p))` |
| `Adjustment::NullableView` | chain (`NullableView`) of `Borrow(as the reference, Conversion, NonNullPayload(Deref(e)))` |
| written `&e` / `&mut e` | `Borrow(_, Written, p)`, or of `Temporary(e)` for a non-place |
| `ReceiverPass::SharedBorrow` / `MutableBorrow` | `Borrow(_, Receiver, Deref^n(p))`, or of `Temporary(e)` |
| `ReceiverPass::Move` | `Move(p)`, or the consumed temporary value itself |
| `ReceiverPass::CopyOut` | `Copy(Deref^n(p))` |

A reborrow is a borrow of a dereference and a copy-out a copy of a dereference, as `&mut *r` and `*r` are in Rust's MIR; the classifications become compositions of four primitives (`Deref`, `Move`/`Copy`, `Borrow`, chains) that MIR lowering handles once each.

Rejected: one node kind per classification (`CopyOut`, `Reborrow`, `BorrowContents`, `NullableView` as leaves), which MIR lowering would have to expand into exactly these projections anyway; and leaving move-versus-copy to MIR lowering by type, as Rust does, because the checker already decides which positions consume (and `E4017` depends on it), so lowering would re-derive the consuming flags.

`BorrowOrigin::Receiver` tells MIR lowering to evaluate the receiver place first and take the borrow after the other arguments (`docs/architecture.md` §4, "Method Call Evaluation Semantics"), which is what two-phase borrows (`borrowck-exclusivity`) need. The origin also lets borrow-checker diagnostics say why a borrow exists.

### D6. Coercion nodes, and a node's type is its slot's type (autonomous)

`Coerce { value, kind }` with `kind` one of `WrapNullable` (`T` to `T?`), `Diverge` (`Nothing` to `T`) and `Subtype` (every other assignability step: `&mut T` to `&T` at any depth, `Nothing?` to `T?`, `T?` to `U?`, `Heap` and function-type variance). The checker adds one wherever a value fits its position by assignability with a different type: in `fit` for `Adjustment::None` when `found != expected`, and at the joins it computes itself, the branches of an inferred `if` and `when` and the operands of `?:`. The invariant: in an error-free tree, every child's type equals the type its slot expects (an argument its parameter, a value its place, a branch its `if`, a tail its block), so MIR lowering never consults subtyping. `T` to `T?` is a representation change (a tag) that MIR must emit.

Rejected: type-directed lowering, where MIR compares a child's type with the slot's type, which re-runs subtyping in another crate. A risk remains for `Subtype`: function-type variance through nullable parameters (`(Int?) -> Unit` given for `(Int) -> Unit`) and `Heap<Int>` given for `Heap<Int?>` change representation, so the backend will need adapters; the node marks every such place (see Risks).

### D7. Safe navigation is a chain with payload projections (autonomous)

`SafeNavigation { chain, origin }` evaluates `chain`, in which `NonNullPayload { nullable }` evaluates `nullable` and, when it is `null`, leaves the innermost enclosing chain with `null`; otherwise it is the place of the payload (or a value, when `nullable` is a value). The chain's type is its body's type made nullable, never nested. `origin` is `Written` for `?.`, or names the conversion of D5 (`NullableView`, `BorrowContents`, `CopyOut`). This is Swift's optional evaluation (`OptionalEvaluationExpr` with `BindOptionalExpr`), whose binding of a place is a place: that is exactly the optional place of `extension-methods` D10. A `?.` whose target is itself a written chain joins it, reusing the inner chain's node, so `t?.lead?.greet()` is one chain whose receiver `NonNullPayload(Field(NonNullPayload(Local t), lead))` is borrowed where it lives. A consumed safe navigation moves or copies its final place inside the chain. Arguments of a safe call are inside the chain, so they are evaluated only when the receiver is not `null`.

Rejected: desugaring `?.` into a `when` over a synthetic local, which copies or moves the payload into that local (wrong for places) and gives the borrow checker spans the user never wrote; a `safe` flag on field and call nodes, which leaves MIR lowering to find where a chain starts and ends, and cannot express that a joined chain's payload is a place.

### D8. Calls, constructions and built-ins (user for positional order and default slots; the rest autonomous)

`Call { callee, arguments, evaluation_order, has_receiver, is_compose }`:

- `Callee::Function(DeclarationPath)` for a top-level function or extension, own or imported; `Callee::Builtin(BuiltinFunction)` for compiler-known functions, only `Clone` today (`print` arrives with `codegen-core` as another variant); `Callee::Value(ExpressionId)` for a function-typed value read in place (a closure binding, a function-typed field, a parameter).
- `arguments` has one `Argument` per parameter in parameter order, the receiver first when `has_receiver`: `Argument::Written(ExpressionId)` or `Argument::Default`. `evaluation_order` lists the parameter indices of the written arguments in source order, the receiver first. Named arguments are evaluated in the order written, as in Kotlin; lowering evaluates defaults after the written arguments. `check_call_arguments` builds both vectors, since it is where names are matched.
- `is_compose` comes from the callee's function type, so `compose-transform` finds every composable call (it adds the composer argument and derives the group key from the call's span).
- A function name read as a value is `FunctionReference(DeclarationPath)`.

Constructions are not calls: `Construct { kind: Struct(NominalId) | Variant { enum_id, index }, fields, evaluation_order }` with fields in declaration order, like Rust's aggregate rvalues. A payload-free variant is a construction with no fields; a payload variant read without a call is `VariantConstructor { enum_id, index }`. A range `a..b`/`a..<b` is `Range { start, end, end_inclusive }` of the built-in `Range` type, since whether the end is included is a property of the runtime value.

Rejected: keeping arguments in source order with a parameter index each, which pushes the matching into every consumer; calls for constructors, which would need a callee identity for something that runs no code.

### D9. Control flow stays structured (follows from the agreed MIR scenario; representation autonomous)

`docs/architecture.md` assigns turning `&&`, `||`, `?.`, `?:`, `when` and `for` into branches to MIR lowering, so the tree keeps them structured:

- `Block { statements, tail }`. A block whose value is discarded (`check_block` with an expected `Unit`) holds its last expression as an `Expression` statement and has no tail, so "the tail is the block's value" holds without exceptions.
- `If { condition, then_branch, else_branch }`, `Logical { operator, left, right }` for `&&`/`||`, `Binary`/`Unary` for the other operators (a typed-tree operator enum without `?:`, ranges, `&` and `*`), `Elvis { nullable, fallback }`.
- `When { subject, branches, exhaustive }`, the subject read in place through its dereferences; each pattern is `Variant { enum_id, index }` (payload-free variants, today's `unit_variant_pattern`), `Value(ExpressionId)` (compared with the subject), `Range { start, end, end_inclusive }`, `Condition(ExpressionId)` (a `when` without subject) or `Else`. `exhaustive` lets MIR mark the fall-through of an `else`-less exhaustive `when` unreachable.
- Statements: `Let { local, initializer }`, `Assign { target, value }` (D10), `Expression { value }`, `While { condition, body }`, `DoWhile { body, condition }` (the condition is evaluated in the body block's scope, so the body's locals live through it), `For { variable, range, body }`.
- `Break { target }` and `Continue { target }` name the loop's `StatementId`, `None` only in an erroneous tree (`E4015`). The builder keeps a stack of open loops, reset inside closures as `loop_depth` is. Loop labels (a ROADMAP item) then only change how the target is found.
- `Return { value }`, of type `Nothing`.

### D10. Assignments are statements over places (follows from `statements-and-loops` and `reference-access`)

`Assign { target, value }` with a place target: a binding (`x = v`; re-pointing when the binding is a reference, `r = &b`), a field path (`u.name = n`, `name = n` through implicit `this` as `Field(Deref(Local this))`), or `Deref` for a write through a reference (`*r = v`, `**rr = 0`). The value is consumed, so a place value is a `Move` or `Copy`. Compound assignment (step 11) will add an operator to this statement rather than a new kind.

### D11. Closures record their captures (autonomous)

`Closure { parameters, body, is_compose, captures }`. A closure's parameters and locals live in the enclosing body's arenas, because a closure is checked inside it. The builder keeps a stack of open closures, each with the number of locals that existed when it opened. Since `LocalId`s are allocated in order, resolving a name to a local whose id is below that number is a capture: it is recorded once, in first-use order with the span of that use, in every open closure it is below, so a nested closure's parent captures what the child does. `this` is a local, so a closure in an extension that reads a field through implicit `this` captures `this`. Top-level declarations are never captures.

The capture mode is not recorded: `borrowck-exclusivity` infers it ("closures capture by inferred borrow") from the uses inside the body, and `closures` (step 17) converts each closure into a function plus an environment of exactly these bindings. Rejected: a separate body per closure, as Rust has, which makes capture discovery a lookup across bodies; MIR lowering can still lower each closure body as its own MIR body.

### D12. Error nodes keep what was checked (autonomous)

`ExpressionKind::Error { children }`, of the error type, wherever the checker cannot give an expression a meaning: an unresolved name, a type name used as a value, a call whose callee or arguments do not match (`E4004`), an unknown field or extension, an invalid assignment target, `this` outside an extension. `children` holds the nodes of the subexpressions that were checked before giving up, so the LSP can still show their types. The builder never reports a diagnostic and never checks a subexpression the checker does not check today, so the set of diagnostics stays the same. A node may have the error type without being an error node (an addition with an erroneous operand); either way, an error-free file has neither (the validator, D14).

### D13. A stable text dump and `prism dump-typed-tree` (autonomous)

`TypedFile::dump()` in `prism-typed-tree/src/dump.rs` prints items in source order, each body with its locals, then its tree, one node per line, two spaces of indentation per level, a role label where a parent has several kinds of children, the type after `:` and the source position as `@line:column`:

```text
module main
struct main::Stack @1:1
  field 0 var size: Int
fun main::Stack.push(mut this, n: Int): Unit @2:1
  local %0 this: &mut Stack (receiver)
  local %1 n: Int (parameter)
  block: Unit @2:31
    assign @2:33
      target: field 0 size: Int @2:33
        deref: Stack @2:33
          local %0 this: &mut Stack @2:33
      value: binary +: Int @2:40
        field 0 size: Int @2:40
          deref: Stack @2:40
            local %0 this: &mut Stack @2:40
        local %1 n: Int @2:47
fun main::main(): Unit @3:1
  local %0 v: Stack (var)
  block: Unit @3:13
    let %0 v @4:5
      construct main::Stack: Stack @4:13
        field size: literal 0: Int @4:19
    expression @5:5
      call main::Stack.push: Unit @5:5
        receiver: borrow mut (receiver): &mut Stack @5:5
          local %0 v: Stack @5:5
        argument n: literal 1: Int @5:12
```

Types print with bare names as diagnostics do; declaration paths print qualified. The exact layout is fixed by the first snapshots and may be tuned during implementation.

`prism dump-typed-tree <file.prism>`, next to `dump-ast`: it resolves the file's workspace, runs Phase 1 over its import graph, checks only that file against its direct imports' ABIs (D15), prints the tree on stdout, and renders any type diagnostics on stderr with exit status 1. Resolve, parse and ABI errors are reported as by `prism check`, with no tree. `mir-lowering` follows the same shape with `prism dump-mir <file.prism>`; `docs/architecture.md` §1, which says `--dump-mir`, is updated to match. Rejected: a `--dump-typed-tree` flag on `prism check`, which checks every file of the graph and stops at the first error; JSON output, which no consumer needs yet and is harder to review in snapshots.

Snapshot fixtures: a fixture case directory may hold `typed-tree.snap` beside its `.prism` files. The runner in `fixture_tests.rs` then also compares `dump_typed_tree(entry)` with it, and `PRISM_BLESS=1 cargo test` rewrites it. The usual `// expect:` header still gives the codes of `prism check`.

### D14. A validator over every passing fixture (autonomous)

`TypedFile::validate()` in `prism-typed-tree/src/validate.rs` returns the violations of the tree's structural invariants: every node reachable from exactly one parent; the operand of `Move`, `Copy`, `Borrow`, `Field`, `Index` and `Assign`'s target is a place; `NonNullPayload` appears only inside a chain; every `LocalId` belongs to its body; every `Break`/`Continue` names an enclosing loop; arguments match the callee's parameter count and, where a slot's type is known (let initializers, assignment values, arguments of declared callees, branches, block tails), the child's type equals it (D6); and, for a tree from an error-free file, no error node or error type. A test in `fixture_tests.rs` builds and validates the tree of the entry file of every `// expect: ok` case, about 300 programs, which checks the builder far more widely than the snapshot fixtures alone. It runs on `examples/` too.

### D15. A file's tree depends only on the file and its imports' ABIs (follows from the deterministic hermetic builds invariant)

`validate_and_extract_abi` in `workspace.rs` is split into `extract_abis(graph)` (Phase 1) and `check_file(graph, abis, path) -> (TypedFile, Vec<TypeError>)` (Phase 2 of one file). `prism check` composes them as today, keeping its order and its stop at the first file with errors. `dump_typed_tree` runs Phase 1 for the graph and Phase 2 for the entry file only, so a dependency's bodies, and their errors, cannot affect it. The pipeline requirement in `compilation-and-abi` states this, and its scenario shows it with a dependency whose body has an error.

### D16. A new `typed-tree` capability, and the pipeline step in `compilation-and-abi` (autonomous)

What the tree records is one subject that `mir-lowering`, `closures` and `compose-transform` will extend, so it gets its own spec, as `references` and `extension-functions` did. The pipeline step, "Phase 2 checks each file into a typed tree from its AST and its imports' ABIs", belongs with Phase 1 in `compilation-and-abi`, which takes the scenario "Typed tree from the type checker" from `docs/architecture.md`. The rest of "Semantic Pipeline After Type Checking" (MIR and borrow checking) stays planned there.

## Risks / Trade-offs

- [The refactor touches every checking function, and a slip could change which diagnostics are reported] → the builder never reports and never checks more than today (D12); every existing fixture keeps its expected codes; tasks land in groups that each keep `cargo test --workspace` green.
- [Snapshot fixtures can be blessed without being read] → each snapshot belongs to a scenario whose THEN states what the tree must show, and the reviewer checks it against the scenario; the validator catches structural errors that a careless bless would hide.
- [`Subtype` coercions that change representation (function-type variance through nullable parameters, `Heap<Int>` as `Heap<Int?>`) need adapters in the backend] → the tree marks every one; `codegen-core` either generates adapters or the type system drops those subtyping rules. The `Heap<T>` covariance question is already on `ROADMAP.md`; function-type variance through nullables is added there on archive.
- [A default value that borrows a temporary (`fun greet(n: &String = "anon")`) cannot simply become a generator function returning the borrow] → the tree records the default with its argument borrow; `mir-lowering` decides where the temporary lives (for example, the generator returns the owned value and the caller borrows it). Added to `ROADMAP.md` on archive as a question for `mir-lowering`.
- [Storing `Type` by value in every node clones types often] → bodies are small and checking already clones types everywhere; the accessor keeps interning possible.
- [Per-body arenas put closure bodies in their parent's arena] → MIR lowering lowers a closure's subtree as its own MIR body, with its captures as its environment.

## Migration Plan

No external users. `prism_typechecker::Type` and friends stay importable through re-exports, and `check_module` now returns the tree, which existing callers may ignore. No fixture changes its expectation; the duplicated `E4001` in an `else`-less `if` disappears from rendered output only. Rollback is a revert.

## Open Questions

- ~~The dump's exact layout (labels, how locals are listed) is settled by the first snapshots.~~ Settled during implementation: the layout of the D13 example, with locals listed after the item's header as `local %n name: Type (kind)`, defaults as `default name:` lines, loops numbered `#n` for their jumps, and a call's or construction's evaluation order printed as `(order: c, a)` only when it differs from parameter order.
- Whether `prism dump-typed-tree` should also accept a directory, dumping every file of a package, can wait for a need.

## Implementation Notes

Decisions and corrections made while implementing, beyond the decisions above.

### D17. Error nodes have the error type everywhere (autonomous)

D12 gives every error node the error type, and the scenario "Error node keeps what was checked" requires it for a call whose arguments do not match. Before this change some failures kept a recovered type: a call with mismatched arguments had its callee's return type, a construction with missing or unknown fields its struct's type, `clone()` with arguments the cloned type, a value moved out of a borrow (`E4017`) its expected type, and a `return` outside a function `Nothing`. They now have the error type, which suppresses follow-on mismatches on the same value (`val x: String = g(1, 2)` reports `E4004` but no longer also `E4002`). No fixture's codes changed, and the rendered diagnostics of every fixture and example are identical except for the rendering fix of D21. Rejected: an error node carrying the recovered type, which keeps more information for hover but makes cascades depend on which failure occurred.

### D18. Calls and constructions record their slots' types (autonomous)

`Call` records its `parameters` (each name, `None` for a function-typed value, and the type its argument has, `this` first for an extension) and `Construct` records each field's name and declared type (`ConstructField`). The validator checks D6's slot rule for arguments and fields from these, and MIR lowering reads a default's parameter or a field's layout without looking up the callee's signature, which for an imported function lives only in the ABI. Rejected: recovering the signature from the declaration path, which needs the import tables in every consumer.

### D19. A call takes over its callee's node (autonomous)

The callee of `f(x)` is checked like any expression first, which builds a `FunctionReference` (or a `VariantConstructor` for `Shape.Circle(1)`). When it turns out to be a declared function or a payload variant, the call (or construction) replaces that node in place rather than leaving it unreachable, so every node of an error-free tree is reached from exactly one parent (D14). Rejected: resolving a callee name without building a node, which would duplicate the name-resolution paths (on-demand inference, visibility, unimported types) of `infer_expr`.

### D20. Comparison operands at one type, and shared borrows passed on are copies (autonomous)

D6 is extended to `==` and `!=`: the operand whose type is assignable to the other's is coerced to it (`Int?` compared with `Int` coerces the `Int`), so MIR lowering compares two values of one type. And an argument place that already is a shared borrow (or another `Copy` value) passed to a reference parameter is an explicit `Copy`, as consumed places are (D5); a non-`Copy` one, such as an optional mutable borrow lent as an optional shared borrow, stays read in place, since the checker does not check it as a move. Rejected: leaving comparison operands at their own types, which pushes subtyping back into MIR lowering.

### D21. Plain fixes and corrections

- **Rendering of optional borrows.** A nullable reference or function type renders as written in source, `(&T)?` and `(() -> T)?`, rather than as `&T?`, which is how a borrowed nullable `&(T?)` renders. This affects the dump and diagnostics alike (one fixture's message changes from "expected '&String?', found '&String?'" to "found '(&String)?'").
- **A range pattern without a subject.** `when { in 0..<10 -> ... }` was accepted with its range unchecked, which left nothing for the tree to hold. Without a subject a pattern is a condition, so the range is now rejected with `E4002` (a plain fix: the scenario "Subjectless when expression condition checking" requires conditions to be `Bool`); `type-system` gains the scenario "Range pattern without a subject rejected" and a regression test.
- **The nested-closure scenario** wrote `{ { a } }`, which is a closure whose body is a block, not two closures; it now uses `run { run { a } }`.
- **Names.** The per-file Phase 2 function of D15 is `typecheck_file`, since `check_file` is already the public entry of `prism check <file>`. Each function and top-level `val` is built in a body builder of its own by `check_function_body` and `check_top_level_var`, which restore the enclosing one; `infer_declaration` also saves it in `BodyContext`, as D1 describes.
- **Variants read through a value** (`h.Green`, accepted by the `type-system` scenario "Only a variant name covers that variant") are a block that evaluates the value as a statement and has the variant as its tail, so the value's node stays reachable.
