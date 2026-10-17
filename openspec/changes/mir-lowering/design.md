# Design: mir-lowering

## Context

See `proposal.md` for motivation. This change is step 6 of "the road to the borrow checker" in `ROADMAP.md`, agreed with the user in the `/opsx:explore` session of 2026-10-17: a `prism-mir` crate lowering the typed tree to a place-based control-flow graph, with a `prism dump-mir <file.prism>` text format for snapshot tests. The planned requirement is `docs/architecture.md` §1, "Semantic Pipeline After Type Checking", scenario "Lowering to MIR": statements assign rvalues to places (a local plus field and deref projections) and mark storage, terminators branch, call, return and drop, and `&&`, `||`, `?.`, `?:`, `when` and `for` become explicit branches. `ROADMAP.md` assigns it four questions from `typed-tree`: where temporaries live, how a default value is evaluated at run time, how a default that borrows a temporary (`fun greet(n: &String = "anon")`) is passed, and how a `(&mut T)?` read in place as a `(&T)?` (typed-tree D20) is reborrowed. The user was not available while this change was proposed, so every question beyond the agreed shape is decided here, marked **(autonomous)**, and logged in `docs/decisions-for-review.md`.

The consumers, each named where a decision serves it:

- `borrowck-moves` (step 7): move paths over places, maybe-moved dataflow, reinitialization by assignment; it also decides where drop flags are needed.
- `borrowck-exclusivity` (step 8): liveness (non-lexical borrows), disjoint field paths, the receiver borrow taken after the arguments, closures capturing by inferred borrow and not escaping.
- `borrowck-elision` (step 9): returned borrows and their sources, borrows of top-level `val`s.
- The Cranelift backend (`codegen-core`, `codegen-aggregates`, `runtime-strings-and-drops`, `closures`, steps 14 to 17): every local typed, enums read through discriminants and switches, nullables as a tagged layout, drop elaboration, closure conversion. Then `hot-reload` (step 18) and `compose-transform` (step 20), which need body identities, call spans and compose flags.

Current state:

- **Typed tree** (`crates/prism-typed-tree`, the `typed-tree` spec). `TypedFile { module, package, items, bodies }`; one `Body` per function and top-level `val` with arenas of `Local`s, `Expression`s and `Statement`s, its `parameters` (`this` first), `defaults` (one optional expression per declared parameter) and `value`. Places (`Local`, `TopLevelValue`, `Field`, `Deref`, `Index`, `NonNullPayload`, `Temporary`), uses (`Move`, `Copy`, `Borrow` with a `BorrowOrigin`), `SafeNavigation` chains, `Coerce` nodes (`WrapNullable`, `Diverge`, `Subtype`), normalized `Call`s (callee, arguments in parameter order with `Default` slots, `parameters`, `evaluation_order`, `has_receiver`, `is_compose`), `Construct`, `Range`, structured control flow, `Closure` with its captures, and `Error`. A node's type always equals its slot's type (D6), so lowering never consults subtyping. `TypedFile::validate(error_free)` checks these invariants.
- **Types.** `Type` lives in `prism-typed-tree/src/types.rs`; a struct or enum is a `NominalId` whose fields, variants and marker live only in the checker (`TypeChecker::types: HashMap<NominalId, TypeDef>` and `markers: HashMap<NominalId, Option<TypeMarker>>`, read by `type_def`, `is_copy` and `is_clone` in `checker.rs`). The built-in `Range` is a `Copy` struct with fields `start` and `end`; whether its end is included is not a field.
- **CLI** (`crates/prism-cli`). `workspace.rs` has `extract_abis` (Phase 1), `typecheck_file` (Phase 2 of one file, returning the tree and errors), `validate_and_extract_abi` (used by `prism check`, stops at the first file with errors) and `dump_typed_tree`. `main.rs` has `run_dump_typed_tree`. `tests/fixture_tests.rs` compares `typed-tree.snap` snapshots (`check_snapshot`, `needs_snapshot`, re-blessed with `PRISM_BLESS`) and validates the typed tree of every fixture and example (`fixture_typed_trees_are_well_formed`, `example_typed_trees_are_well_formed`).

## Goals / Non-Goals

**Goals:**
- Every error-free file lowers to MIR in which every value's storage, every drop, every branch and every call is explicit, so the borrow checker reads nothing but MIR and imported ABIs.
- The MIR is the backend's input too: every local typed, places with typed projections, discriminant reads and switches, explicit nullable wrapping.
- The four questions `typed-tree` left open are answered, each with a scenario and a snapshot fixture.
- No behaviour change: `prism check` reports exactly what it reports today; the only observable additions are `prism dump-mir` and the imported types in the typed-tree dump.

**Non-Goals:**
- Borrow checking, use-after-move, liveness and drop flags (steps 7 to 9), and drop elaboration (`runtime-strings-and-drops`).
- Capture modes and closure conversion (`borrowck-exclusivity`, `closures`).
- Integer overflow and division by zero: no assertion terminators; `codegen-core` decides (see Open Questions).
- Unwinding: Prism has no panics that unwind, so calls and drops have no cleanup edges.
- Layouts, optimizations, constant folding, SSA form, caching or serializing MIR, and the `compose fun` transform.

## Decisions

Decisions marked **(user)** were agreed in the exploration session; **(autonomous)** ones were made while proposing this change and are open to review. The others follow from these or from existing rules.

### D1. A `prism-mir` crate, lowering only error-free typed trees (user for the crate and the CFG; the rest autonomous)

`prism-mir` depends on `prism-typed-tree` and `prism-ast` (spans, literals). Its entry point is `prism_mir::lower_file(file: &TypedFile) -> MirFile`, which requires a tree from a file without type errors: the tree's error-free invariants (no error node, slot types equal, `Break`/`Continue` targets present) are its preconditions, and a violation is an internal compiler error (a panic naming the node), not a diagnostic. This follows `docs/architecture.md` ("WHEN a file has no type errors"): lowering a tree with error nodes would need a recovery story for nodes without meaning, and no consumer needs MIR for a broken file (the LSP reads the typed tree).

`prism check` lowers every file that typechecks, right after `typecheck_file` in `validate_and_extract_abi`, and discards the MIR for now. Lowering reports nothing, so `prism check`'s output is unchanged, but every checked program then exercises lowering, and `borrowck-moves` only has to call the borrow checker on the MIR it already gets. Rejected: lowering only in `dump-mir`, which would leave lowering crashes unnoticed by `prism check` until step 7.

### D2. A small, Rust-like MIR (user for the shape; the exact sets autonomous)

```rust
pub struct MirFile { pub module: String, pub types: TypeTable, pub bodies: Arena<BodyId, Body> }
pub enum BodyOwner {
    Function(DeclarationPath),
    Value(DeclarationPath),                                  // a top-level `val`'s initializer
    Default { function: DeclarationPath, parameter: usize }, // D11; `parameter` counts declared parameters
    Closure { parent: BodyId, index: usize },                // D13
}
pub struct Body {
    pub owner: BodyOwner,
    pub is_compose: bool,
    pub locals: Arena<LocalId, LocalDeclaration>, // _0 the return place, then the arguments, then captures
    pub argument_count: usize,
    pub capture_count: usize,
    pub blocks: Arena<BlockId, BasicBlock>,        // bb0 is the entry
    pub span: Span,
}
pub struct LocalDeclaration {
    pub ty: Type, pub kind: LocalKind, pub mutable: bool,
    pub name: Option<String>, pub binding: Option<prism_typed_tree::LocalId>, pub span: Span,
}
pub enum LocalKind { Return, Argument, Capture, User, Temporary }
pub struct BasicBlock { pub statements: Vec<Statement>, pub terminator: Terminator }
pub enum StatementKind { Assign(Place, Rvalue), StorageLive(LocalId), StorageDead(LocalId), Read(Place) }
pub enum TerminatorKind {
    Goto(BlockId),
    SwitchInt { value: Operand, targets: Vec<(u64, BlockId)>, otherwise: BlockId },
    Call { callee: Callee, arguments: Vec<Operand>, parameters: Vec<Type>,
           destination: Place, target: Option<BlockId>, is_compose: bool },
    Drop { place: Place, target: BlockId },
    Return,
    Unreachable,
}
pub struct Place { pub local: LocalId, pub projections: Vec<Projection> }
pub enum Projection { Deref, Field(usize), Payload, Index(LocalId) }
pub enum Operand { Copy(Place), Move(Place), Constant(Constant) }
pub enum Constant {
    Int(i64), Float(f64), Bool(bool), Char(char), String(String), Unit, Null(Type),
    Function(DeclarationPath), VariantConstructor { enum_id: NominalId, index: usize },
    TopLevelValue(DeclarationPath),                          // D5: a `&T` to the value
}
pub enum Rvalue {
    Use(Operand),
    Borrow { mutable: bool, origin: BorrowOrigin, place: Place },
    Binary(BinaryOperator, Operand, Operand),
    Unary(UnaryOperator, Operand),
    Discriminant(Place),                                     // enum: variant index; nullable: 0 null, 1 present
    Aggregate(AggregateKind, Vec<Operand>),                  // struct, variant; operands in declaration order
    WrapNullable(Operand),                                   // T to T?: sets the tag
    Subtype(Operand),                                        // any other subtyping step, to the destination's type
    Closure { body: BodyId, captures: Vec<Place> },
}
pub enum Callee { Function(DeclarationPath), Builtin(BuiltinFunction), Value(Place), Default { function: DeclarationPath, parameter: usize } }
pub enum BorrowOrigin { Written, Argument, Receiver, Conversion, Operator, Default } // `Operator` was `Comparison`, see N1
```

Every statement and terminator carries the `Span` of the typed-tree node it comes from; storage-dead markers and drops at a scope's end carry the span of that scope (block, statement or branch). Operators, `BuiltinFunction`, `DeclarationPath`, `NominalId` and `Type` are the typed tree's.

What it keeps from Rust's MIR: places as a local plus projections, operands that say move or copy, borrows as rvalues, storage markers, `Drop` and `Call` as terminators, `SwitchInt` on integers. What it leaves out, because Prism does not need it yet: unwinding edges (Prism has no unwinding panics), `Assert` (overflow semantics are undecided), casts other than subtyping, `FakeRead` kinds (one `Read`), user type annotations, scopes for debuginfo, and promoted constants. `Rvalue::Subtype` marks every representation-preserving or -changing subtyping step the tree's `Coerce { kind: Subtype }` marks, so the backend finds the adapters `typed-tree` D6 warned about; `WrapNullable` is separate because it always changes the representation. `Drop` is a terminator, as the planned scenario says, so drop elaboration can turn one into a flag test or a call to drop glue without splitting blocks.

Rejected: an SSA form, which the borrow checker's place-based analyses would have to undo, and which Cranelift builds itself from variables; a tree-shaped "lowered tree" with explicit drops, which leaves every dataflow analysis to rebuild the CFG.

### D3. Bodies, locals and their identities (autonomous)

One MIR body per function with a body, per top-level `val`, per parameter default (D11) and per closure (D13), each with a `BodyOwner`. Bodies are ordered by item. Within an item comes the function's own body, then its default generators in parameter order; each of these bodies is followed by the closures lowered inside it, in the order their expressions are lowered. A closure is named by its parent body and its index among the parent's closures (`main::g#closure0`), so `hot-reload` and `compose-transform` can key bodies symbolically, as typed-tree D4 does for declarations.

Locals are numbered Rust's way: `_0` is the return place, `_1` to `_n` the arguments (`this` first for an extension), then a closure's capture locals, then user bindings and temporaries in order of creation. A local records the typed-tree binding it stands for (`binding`, `name`), so borrow-checker diagnostics can name `s` in "use of moved value `s`" and the LSP can map back. A typed-tree binding gets its MIR local when its `Let`, `for` or parameter is lowered, so a binding in code that is never lowered (D9) has none.

### D4. The typed tree carries its file's type table (autonomous)

MIR lowering must know whether a type is `Copy` (to drop it or not, D6) and what a field's type is (to type projections); the validator, the borrow checker's field paths and the backend's layouts need the fields of imported structs. Today only the checker knows them. `prism-typed-tree/src/types.rs` gains

```rust
pub struct TypeTable { definitions: BTreeMap<NominalId, TypeDef>, markers: BTreeMap<NominalId, Option<TypeMarker>> }
```

with `definition`, `marker`, `is_copy` and `is_clone` moved there from `checker.rs` unchanged (the error type still counts as `Copy`). The checker's `types` and `markers` maps become one `TypeTable`, its `is_copy` and `is_clone` delegate to it, and `check_module` moves it into `TypedFile::types`. It holds what the checker knows: the file's own structs and enums, those its direct imports export (from their ABIs), markers of types an imported signature mentions, and the built-in `Range`. `TypedFile::dump` lists the imported entries after the `module` line (`import struct models/user::User : Clone` with its fields), so the scenario has a snapshot; own declarations already print as items. `MirFile::types` is a copy, so borrow checking and code generation need only the MIR.

The built-in `Range` gets a third field in the MIR's copy only: `endInclusive: Bool`, which a `Range` construction sets and a `for` loop reads (D8). The type system keeps exposing `start` and `end` only. Rejected: a user-visible `endInclusive` field (a language change no one asked for); normalizing half-open ranges to closed ones, which changes what `r.end` reads.

Rejected: lowering asking the checker (the checker is gone once the tree exists, and a later stage would need the checker crate); field types stored in each projection (Rust's `Field(index, ty)`), which still leaves `Copy` markers to find and duplicates the table in every place.

### D5. Places and operands from typed-tree places (follows from typed-tree D5; constants autonomous)

Each tree place lowers to a MIR place:

- `Local(id)` is the binding's local, or, in a default generator, the dereference of the argument holding its borrow (D11), or, in a closure, its capture local (D13).
- `Field`, `Deref` and `Index` add a projection (`Index` first evaluates its index into a temporary local, as Rust requires). `Deref` covers `&`, `&mut` and `Heap<T>`.
- `NonNullPayload(nullable)` tests the tag of the nullable's place and adds `Payload` on the non-null edge (D8); a nullable that is not a place is evaluated into a temporary first.
- `Temporary(value)` evaluates `value` into a new temporary local in the current temporary scope (D7) and is that local.
- `TopLevelValue(path)` is `(*_t)`, where `_t = const &path` holds a `Constant::TopLevelValue` of type `&T`. Places stay a local plus projections (Rust lowers statics the same way), a top-level `val` is immutable through a shared borrow, and `borrowck-elision` recognizes borrows derived from such a constant as valid everywhere.

`Move(p)` and `Copy(p)` become `Operand::Move` and `Operand::Copy`; literals and function references become constants, a string literal being a constant that evaluates to a fresh owned `String` (the `ownership` spec). A place read in place where an operand is needed is a `Copy` when its type is `Copy`; a non-`Copy` place read in place is compared through a borrow (D14), called as a callee place (D10), or read by a `Read` statement when it is a discarded statement or the subject of a `when` with only `else`, so the move checker sees the use. Any other value is evaluated into a temporary and moved out of it. `Borrow { mutable, origin, place }` becomes `Rvalue::Borrow` with the same origin. `Construct` and `Range` become aggregates, fields in declaration order after evaluating the written values in source order (`evaluation_order`).

### D6. Storage markers and scope-exit drops at lowering time; refinement later (autonomous)

Lowering keeps a stack of scopes (bodies, blocks, statements and the temporary scopes of D7). Each scope records the locals and temporaries it owns in creation order. When a local or temporary is created, it gets `StorageLive`; when its scope ends, by falling through or by any jump out of it (`return`, `break`, `continue`, a chain's null edge), lowering emits, for each owned local in reverse order, a `Drop` terminator if its type needs a drop, then `StorageDead`. The arguments of a body are dropped when it returns, after its locals; the return place and capture locals are never dropped or marked.

A type **needs a drop** unless it is `Copy`, a mutable borrow `&mut T`, or a nullable of either. So `String`, `Heap<T>`, function types, opaque types and every non-`Copy` struct, enum or nullable of one get a drop, even when the struct holds only `Int`s; whether a drop does anything is a layout question for `runtime-strings-and-drops`, which also needs the drop-relevant contents of imported types nested in other imported types (see Risks).

An assignment to a place whose type needs a drop evaluates the value, then drops the place, then writes it (`*r = v` drops the old referent, as `docs/architecture.md` §4 "Writing through a reference drops the old referent" requires).

Drops are not refined by moves at lowering time: a local moved into a call is still dropped at its scope's end. `borrowck-moves` computes maybe-moved places by dataflow on this MIR, and drop elaboration (`runtime-strings-and-drops`, step 16, "drop elaboration on MIR") turns each drop into nothing, a static drop, or a drop guarded by a flag, as Rust's `ElaborateDrops` does. The one refinement lowering does make is for temporaries it creates and moves itself in the same block (a call argument, an aggregate operand, an assigned value): such a temporary is recorded as moved when its operand is consumed and gets no drop (Rust's `record_operands_moved`), so snapshots do not fill with drops that are statically dead. If a diverging expression interrupts before the move (`f(make(), return 1)`), the temporary is still scheduled and the `return` path drops it.

Why at lowering time: the scope structure exists only during lowering; afterwards a drop's position would have to be reconstructed from storage markers. And the borrow checker must see drops as uses (`borrowck-exclusivity`: a value dropped while borrowed is a conflict). Rejected: inserting drops after borrow checking, which hides those conflicts; inserting only "definitely needed" drops by running move analysis inside lowering, which duplicates `borrowck-moves`.

### D7. Temporary scopes and lifetime extension (autonomous; settles the `typed-tree` follow-up and the `val r = &make()` item)

The typed tree says a temporary lives "until the end of the enclosing statement". Taken literally, a temporary created in only one branch, or in every iteration of a condition, would need a drop flag at the statement's end. Instead, as in Rust's terminating scopes (with Rust 2024's tail-expression and `if let` rules), a temporary lives until the end of the innermost enclosing **temporary scope**, which is:

- a statement (`val`/`var`, assignment, expression statement);
- a block's tail, which ends before the block's locals are dropped;
- the value of a function, top-level `val`, closure or default body;
- a part evaluated conditionally or repeatedly: the condition of `if`, `while` and `do`/`while`, the range of `for`, each pattern test of a `when`, each branch of `if` and `when`, the right operand of `&&` and `||`, the fallback of `?:`, and the rest of a `?.` chain after its first payload test.

So every temporary is created unconditionally within its scope and never needs a drop flag, conditions do not hold temporaries across their branches (`if (check(make()))` drops the `String` before branching), and the semantics of `docs/architecture.md` §4 hold: `greet(make())` keeps the `String` until `greet` returns and the statement ends, and a receiver temporary (`make().greet(next())`) lives until the statement ends.

A `when` subject is evaluated once, in the enclosing scope, so it lives through the pattern tests. The nullable operand evaluated by a chain's first payload test also belongs to the enclosing scope; anything the chain evaluates after that test belongs to the chain's scope, which a `null` from a later test leaves like a `break`.

**Lifetime extension.** A `val` or `var` whose initializer is (up to `Coerce` nodes) a written borrow of a place rooted at a temporary (`val r = &make()`, `val n = &make().name`) owns that temporary: it belongs to the binding's block scope, created just before the binding, so it is dropped just after it. This is Rust's rule for `let` restricted to the direct operand of `&`; without it, `val r = &make()` typechecks today and would always be a dangling-borrow error in `borrowck-exclusivity`. Extension through branches (`val r = if (c) &a() else &b()`) and through assignments (`r = &make()`) is not done; both remain errors for the borrow checker and possible relaxations.

Rejected: one scope per statement with drop flags for conditional temporaries (more flags, and borrows of a branch's temporary would outlive the branch); Rust 2021's rule that a tail's temporaries outlive the block's locals, which Rust itself reversed because `{ val u = make(); u.name.len() }`-style code was rejected.

### D8. Control flow lowering (user for "become explicit branches"; shapes autonomous)

Lowering is destination-passing, as rustc's `expr_into_dest`: a value-producing construct lowers into a destination place, each branch assigning it.

- **`if`** evaluates its condition (a temporary scope) into a `Bool` operand, `SwitchInt` with `0` to the `else` block and otherwise to the `then` block, both joining. A branch of type `Nothing` (a `Coerce { kind: Diverge }`) does not join.
- **`&&` / `||`** evaluate the left operand and switch; only the edge that needs the right operand evaluates it (in its own temporary scope); the other assigns `false` (`&&`) or `true` (`||`).
- **`?:`** evaluates its consumed nullable into a temporary, reads its discriminant and switches: on `1` the payload is moved (or copied) out into the destination, on `0` the fallback is evaluated (its own scope). The temporary needs no drop afterwards (its payload was moved, or it is `null`), so it is recorded as moved.
- **`?.` chains.** The chain's destination is the chain's type `T?`. Each `NonNullPayload` reads its nullable's discriminant and switches; the `0` edge leaves the chain (dropping the chain scope's temporaries created so far, D7) to a block that assigns `const null`; the `1` edge continues with the `Payload` place. The chain's body lowers into the destination, wrapped with `WrapNullable` unless its type already is nullable. Because a `?.` that joins a chain reuses its payload places (typed-tree D7), `t?.lead?.greet()` borrows `(((_1 as payload).0) as payload)` where it lives.
- **`when`** evaluates its subject once (in place if it is a place, else into a temporary). If every pattern is a payload-free variant or `else`, it reads the discriminant once and emits one `SwitchInt` over the variant indices, its `otherwise` edge going to the `else` branch; otherwise it tests branches in order, and within a branch its patterns in order, each test a temporary scope: a value pattern is an equality (D14), a range pattern `start <= s && s <= end` (`<` for a half-open end), a condition the condition itself, a variant pattern a discriminant comparison. When nothing matches, an exhaustive `when` without `else` reaches an `Unreachable` terminator (the tree's `exhaustive` flag), and a non-exhaustive statement reaches the join.
- **`while`** checks its condition at the loop head; **`do`/`while`** runs its body first and evaluates its condition in the body block's scope (the typed tree's rule), dropping the body's locals before the back edge or the exit.
- **`for (i in range)`** evaluates the range once (a temporary scope) into a hidden local, reads `start`, `end` and `endInclusive`, and exits at once if the range is empty (`start > end`, or `start >= end` for a half-open range). Otherwise it computes the last element (`end`, or `end - 1`, which cannot overflow once the range is known non-empty), binds `i` to the current value in a fresh storage scope per iteration, and in the latch, which is `continue`'s target, exits when the current value equals the last element and increments it otherwise. No addition can overflow, even for `0..Int.MAX`, which `docs/architecture.md` §4's "Range bounds evaluated once" also needs.
- **`break`, `continue` and `return`** exit every scope between them and their target, emitting those scopes' drops and storage markers on the jump's path, then jump to the loop's exit block, its latch or condition, or the body's return block. `return v` first assigns `v` to `_0`. The return block drops the arguments and returns.

### D9. Diverging expressions and unreachable code (autonomous)

After lowering an expression of type `Nothing` (`return`, `break`, `continue`, a `Diverge` coercion's operand, a call of a function returning `Nothing`), there is no current block, and lowering skips the rest of the enclosing block's statements and tail, creating no locals for them. Blocks are created on demand (a loop's exit only when something reaches it, a join only when a branch falls through), so every block of a body is reachable from its entry, which the validator checks. A call whose callee returns `Nothing` has `target: None`.

So code after `return` is not borrow-checked, as in Rust (whose borrow checker runs after unreachable blocks are removed), and it was already typechecked. A warning for unreachable code waits for the warnings channel (`ROADMAP.md`). Rejected: lowering dead code into blocks without predecessors, which the borrow checker would then have to skip or report on.

### D10. Calls (follows from typed-tree D8 and `docs/architecture.md` §4; operand rules autonomous)

A tree `Call` becomes a `Call` terminator, its destination a temporary of the call's type (or the enclosing destination), its target the block after the call.

- **Callee.** `Callee::Function(DeclarationPath)` for own and imported functions and extensions alike: the same symbolic path the typed tree and the ABI use, so a caller needs only the ABI, and `hot-reload` can key its function-pointer table by it. `Callee::Builtin(Clone)` for `clone()`. `Callee::Value(place)` for a function-typed value read in place (a non-place callee is first evaluated into a temporary); the borrow checker treats the call as a shared use of that place. `Callee::Default { function, parameter }` calls a default generator (D11).
- **Operands** are in parameter order, one per parameter, the receiver first; `parameters` repeats the parameter types from the tree's `Call` (typed-tree D18), so the validator and the backend need no signature lookup. A by-value argument is moved out of its temporary; a borrowed argument is the borrow's temporary.
- **Evaluation order.** The receiver's place is evaluated first (its index operands, or the temporary holding a receiver that is not a place); then the written arguments in source order (`evaluation_order`), each into a temporary; then the receiver is borrowed or moved (a `Receiver`-origin borrow is taken here, after the arguments); then the omitted arguments are evaluated in parameter order by calling their generators (D11); then the call is made. This is `docs/architecture.md` §4's "Receiver evaluated before the arguments" and "A mutable receiver is borrowed after the arguments": in `v.push(v.size())`, `size` runs before `&mut v` exists, so `borrowck-exclusivity` accepts it by liveness alone, with no special two-phase rule.
- **Compose.** `is_compose` is copied from the tree's call, and `Body::is_compose` is set for `compose fun` bodies and composable closures, so `compose-transform` finds every composable call and its span (its group key) on the MIR.
- **Safe calls** lower inside their chain (D8), so no argument is evaluated when the receiver is `null`.

### D11. Default values are generator bodies; a reference parameter's default gets a caller-owned slot (autonomous; settles two `typed-tree` follow-ups)

Each declared parameter with a default gets a generator body, owned by `Default { function, parameter }` and lowered from the tree's `Body::defaults[parameter]`, in the function's own module (Swift's default argument generators, as typed-tree D3 anticipated). A caller never sees a default's expression, which may call the callee's private declarations; it calls the generator by the function's path and the parameter's index.

**Signature, derived from the callee's signature alone.** The generator's arguments are a shared borrow `&T` of each earlier argument the call passes, the receiver first: `this` and every parameter before this one, written or itself defaulted (defaults are evaluated in parameter order, so they are ready). It returns the parameter's type. In the generator, the binding of `this` or an earlier parameter is the place `*_i` (D5). Passing borrows rather than values leaves every argument with the caller until the call itself; passing all earlier arguments, not only the ones the default reads, keeps the signature a function of the callee's signature, so a caller in another file derives it from the ABI and a change to a default's expression never changes a caller (it already changes the ABI hash, `compilation-and-abi` "Changing a default parameter value").

**A default that borrows a temporary.** `fun greet(n: &String = "anon")` cannot return a borrow of its own temporary, which would die when the generator returns, while the `references` spec says a default is an argument and an argument's temporary lives until the end of the caller's statement. So a generator whose parameter has a reference type (`&R`, `&mut R`, `(&R)?` or `(&mut R)?`) takes one more argument: a mutable borrow of a **slot** of type `R?` that the caller creates as `null` in its statement's temporary scope and drops at the statement's end. In the generator, the temporary at the root of the default's borrowed place (under its argument or written borrow, coercions and `BorrowContents` chain) is the slot's payload: the value is assigned to `*slot`, wrapped unless it already is an `R?` (assigned without a drop, since the slot is `null` on entry), and the borrow is taken of `(*slot) as payload`, through a tag test when the value was nullable. A default that borrows anything else (`n: &User = &DEFAULT`, an earlier parameter `n: &String = other`) ignores the slot and returns its borrow. The returned borrow's sources are the generator's input borrows, which is what `borrowck-elision`'s rule for several input borrows gives it, and the caller's slot stays borrowed until the argument's last use in the statement.

Rejected: recording in the ABI whether each default borrows a temporary (Phase 1 extracts the ABI from the AST alone, and the classification needs type checking); Kotlin's `f$default` wrapper taking every argument and a bitmask of omitted ones, which needs placeholder values for omitted reference parameters that MIR's typed locals cannot express; inlining the default's expression at each call site, impossible across files; rejecting a default that borrows a temporary, which is the natural way to write `n: &String = "anon"` when string literals are owned values.

A default that moves an earlier parameter (`fun f(a: String, b: String = a)`) typechecks today; in its generator it is a move out of a borrow, which `borrowck-moves` rejects. Whether the type checker should reject it first is a follow-up.

### D12. An optional mutable borrow lent as an optional shared borrow is reborrowed (autonomous; settles the `typed-tree` D20 follow-up)

The typed tree passes a `(&mut T)?` place to a `(&T)?` parameter as a `Coerce { kind: Subtype }` over the place read in place, since moving it would make the binding unusable. Lowering a `Subtype` coercion of a place read in place reborrows it: it reads the place's discriminant and switches; on `1` the destination is `WrapNullable(&(*(place as payload)))`, a shared `Argument`-origin borrow; on `0` it is `const null`. So `show(m); show(m)` leaves `m` usable, as the reborrow of a non-optional `&mut T` does (`references`, "Passing a Mutable Borrow Reborrows It"), and the borrow checker sees a shared borrow of the referent, which ends with the call. A `Subtype` coercion of a consumed value is `Rvalue::Subtype` of the moved or copied operand. Any other `Subtype` over a place read in place is an internal error, and the typed-tree validator already guarantees there is none.

### D13. Closures are separate bodies with capture locals; capture modes stay with the borrow checker (autonomous)

Each closure is its own body (`Closure { parent, index }`), lowered from the closure's subtree in its parent's arenas: its parameters are its arguments, and each capture the tree records (typed-tree D11) is a **capture local**, after the arguments and in capture order, of the captured binding's type. A capture local stands for the enclosing binding itself: the closure body reads, writes, borrows or moves it as if it were the binding, it is never marked or dropped by the closure, and in the enclosing body the closure value is `Rvalue::Closure { body, captures }`, whose captures are the places of the captured bindings there (a local, a generator's `*_i`, or the enclosing closure's own capture local).

So the MIR says exactly what each closure does with each capture without deciding how it is captured. `borrowck-exclusivity` infers each capture's mode from the closure body's uses of its capture local (any mutable use makes it a mutable borrow), treats the closure value as holding those borrows of the captured places for as long as it is live, and rejects a capturing closure that escapes. `closures` (step 17) then converts each closure into a function taking an environment and rewrites capture locals into projections of it. Rejected: an environment argument now (`*(*_env).k`), whose field types depend on the capture modes, which are not decided; inlining closure bodies into their parents, which the borrow checker cannot check as calls through a function value.

### D14. Comparisons of non-`Copy` values borrow them (autonomous)

Operators read their operands in place (typed-tree D5), and arithmetic, ordering and logic operands are `Copy`. `==` and `!=` may compare a `String`, a struct or a nullable of a non-`Copy` type; moving or copying those is wrong. So `==` and `!=` over a non-`Copy` type compare shared borrows (`Operator` origin, see N1) of their operands: a place is borrowed where it lives, a value is evaluated into a temporary and borrowed. For nullable operands both sides are optional borrows `(&T)?`: a nullable place is viewed through a tag test and a borrow of its payload, and a `?.` chain read in place whose final node is a place is borrowed inside the chain (`u?.name == "a"` compares the `(&String)?` of `u`'s `name` with a view of the temporary `"a"`), so no field is moved. A `when` whose subject is a non-`Copy` value compared by value patterns evaluates the borrow once. Comparing with the `null` literal (after its coercion) reads the discriminant instead (`Discriminant(p) == 0`), so `s == null` reads only the tag. The backend implements equality over `&T` and `(&T)?` by type.

### D15. A stable text dump and `prism dump-mir` (follows from typed-tree D13; layout autonomous)

`MirFile::dump()` in `prism-mir/src/dump.rs` prints bodies in order, each with its header, its locals and its blocks, one statement or terminator per line with its `@line:column`:

```text
module main
fun main::f(): Int @3:1
  local _0: Int (return)
  local _1: String? (temporary)
  local _2: &mut String? (temporary)
  local _3: &String (temporary)
  bb0:
    storage_live _1 @3:16
    _1 = const null @3:16
    storage_live _2 @3:16
    _2 = &mut _1 (default) @3:16
    storage_live _3 @3:16
    _3 = call default main::greet n(move _2) -> bb1 @3:16
  bb1:
    _0 = call main::greet(move _3) -> bb2 @3:16
  bb2:
    storage_dead _3 @3:16
    storage_dead _2 @3:16
    drop _1 -> bb3 @3:16
  bb3:
    storage_dead _1 @3:16
    return @3:1
default main::greet n(_1: &mut String?): &String @2:24
  local _0: &String (return)
  local _1: &mut String? (argument)
  bb0:
    (*_1) = wrap const "anon" @2:24
    _0 = &(*_1 as payload) (argument) @2:24
    return @2:24
```

Places print as `_1.name` for fields (names from the type table), `(*_1)` for dereferences, `(_1 as payload)` for payloads and `_1[_2]` for indices; operands as `move _1`, `copy _1` or `const …`; borrows with their origin. The exact layout is fixed by the first snapshots and may be tuned during implementation.

`prism dump-mir <file.prism>` resolves the file's workspace, runs Phase 1 over its import graph, typechecks only that file against its direct imports' ABIs, and prints its MIR. With type errors it renders them and exits with status 1, printing no MIR (D1). Resolve, parse and ABI errors are reported as by `prism check`. A fixture case may hold `mir.snap`; `fixture_tests.rs` compares `dump_mir(entry)` with it, `PRISM_BLESS=1` rewrites it, and every case of the `mir` spec and every case whose name starts with `mir` must hold one.

### D16. A MIR validator over every passing fixture and example (autonomous)

`MirFile::validate() -> Vec<String>` in `prism-mir/src/validate.rs` checks: every block has a terminator and is reachable from `bb0`; every jump targets an existing block; every local used is declared, arguments are `_1` to `_n` and captures follow them; every place's projections are well typed (a `Deref` of a reference or `Heap<T>`, a `Field` index in range of a struct, a `Payload` of a nullable, an `Index` of a `String` by an `Int` local); every assignment's rvalue has the place's type, except `Rvalue::Subtype`, whose operand must differ from it; `Copy` operands have `Copy` types; aggregates match their fields; a call's operands have its parameter types, its destination its return type, and it has no target exactly when it returns `Nothing`; `SwitchInt` reads a `Bool` or `Int`; a `Drop` only drops a place whose type needs a drop; capture locals are only in closure bodies and are never marked or dropped; and, by a forward dataflow, every local except the return place, arguments and captures is storage-live at each use, never marked live twice without dying in between, and dead on every path to `Return`. A test in `fixture_tests.rs` lowers and validates the MIR of every fixture whose entry typechecks (about 350 programs) and of every example except `examples/demo/ui`, as typed-tree D14 does for trees.

### D17. A new `mir` capability; the pipeline step in `compilation-and-abi`; the type table in `typed-tree` (autonomous)

What the MIR holds is one subject that the borrow checker, drop elaboration, closure conversion and the backend will extend, so it gets its own spec, as `typed-tree` did. The pipeline step ("Phase 2 lowers each file that typechecks to MIR from its typed tree alone") belongs with "Phase 2 Produces a Typed Tree per File" in `compilation-and-abi`, which takes the scenario "Lowering to MIR" from `docs/architecture.md`; the scenario "Borrow checking on MIR" stays planned there. The type table is a property of the typed tree, so its requirement goes to `typed-tree`.

### Implementation notes

Found while implementing; D2, D14 and D15 are refined by them, and the decisions marked **(autonomous)** are logged in `docs/decisions-for-review.md`.

- **N1. Every operator over non-`Copy` operands borrows them (autonomous).** D14 assumed only `==`/`!=` read non-`Copy` operands, but string concatenation (`a + b`) does too, and the typed tree reads its operands in place. Any binary operator over a type that is not `Copy` now reads shared borrows of its operands (optional borrows for nullables), so the borrow origin is named `Operator` instead of `Comparison`; the validator types arithmetic over `&T` operands as `T`. The `mir` delta gains the scenario "String concatenation borrows its operands". Rejected: a separate rvalue or built-in call for concatenation, which the backend can still choose by operand type; moving the operands, which the typed tree does not allow.
- **N2. A condition's result has its own scope, dissolved by the branch (autonomous).** The `Bool` that a condition or pattern test evaluates must outlive the condition's temporary scope (which ends before the branch, D7) but not survive into the branches. It lives in a scope of its own that a jump out of the condition exits normally, and that the branch dissolves by marking it dead at the start of both successors, as Rust does. A condition that is a binding is switched on directly.
- **N3. `Unit` destinations are written (autonomous).** A block without tail, an `if` without `else` and a `Unit` `when` lowered into a destination assign it `const ()`, so `_0` and every `Unit` binding is initialized on every path, which `borrowck-moves`' initialization analysis needs. Discarded values (expression statements, branches of a discarded `if`) have no destination at all.
- **N4. Data-model and dump details.** `StatementKind::Assign` boxes its rvalue (the rvalue is large); `BodyOwner::Default` and `Callee::Default` also carry the parameter's name, so a dump names a generator at a call site in another file. Blocks that a `goto` is the only way into are merged into their predecessor once a body is lowered, so joins and returns reached by one path add no block. A `for` over a `Range` construction reads its end-inclusiveness from the construction; over any other range value it switches on `endInclusive`. A `when` branch whose variants all match earlier branches gets no block, and ordered tests stop after `else`, so every block stays reachable. A written argument goes into a temporary unless it is a constant that no default generator borrows (D10). A chain whose body is wrapped evaluates the body into a temporary created before its first test, in the enclosing scope.
- **N5. A `continue` in a `do`/`while` leaves the body's later locals live and unassigned (autonomous; found in review).** The condition of a `do`/`while` sees the body's top-level locals (the `loops` spec), and `continue` jumps to it, so a `continue` that skips the declaration of `x` reaches a condition that may read `x`. The `continue` keeps the body's scope open (its earlier locals are not dropped) and, once the body is lowered, marks live without a value each body local declared after it, so every local is storage-live in the condition on every path and the body scope ends on both of its edges as before. Reading such a local is then a read of an uninitialized value, which `borrowck-moves`' initialization analysis must reject (Kotlin reports "variable must be initialized" there); a drop of one is a maybe-uninitialized drop for drop elaboration. Rejected: dropping the body's locals on the `continue` path, which would end values the condition still reads; a type-checker error for every `continue` before a declaration, which also rejects conditions that read nothing it skipped.
- **N6. An operand reading a place holds its value from when it was evaluated (found in review).** An operator, range or construction collects its operands before its rvalue, and a `Copy` or `Move` of a place is an operand that reads the place only at the rvalue, so `n + inc(&mut n)` added the incremented `n`. When a later operand (or, for a `when`'s `Copy` subject, a later pattern) may write a place, because it calls a function or runs a statement, an earlier operand reading a place not rooted at a lowering temporary is first copied or moved into a temporary, as rustc's `as_operand` always does. Operands followed only by reads stay in place, so most snapshots are unchanged. Call arguments already go into temporaries (D10), and the receiver's value is still read after the arguments, as D10 decided.

## Risks / Trade-offs

- [Lowering is a large piece of code with no consumer yet, so mistakes could hide until the borrow checker] → every scenario has a snapshot whose THEN states what the MIR must show; the validator runs over every passing fixture and example; `prism check` lowers every checked file, so a crash shows up in any test.
- [Drops on every non-`Copy` value, unrefined, make snapshots longer and are wrong without elaboration (a moved value is dropped again)] → MIR is not executed before `runtime-strings-and-drops` elaborates drops, and `borrowck-moves` gives it the maybe-moved dataflow; temporaries lowering moves itself carry no drop, which removes the most common dead drops.
- [Whether an imported struct owns resources can depend on types nested in it from modules the file does not import, which its direct imports' ABIs do not describe] → MIR drops every non-`Copy` value, which needs only markers; `runtime-strings-and-drops` and `codegen-aggregates` must get layouts and drop glue from somewhere (the ABI exporting a "needs drop" bit and a layout, or symbolic drop-glue functions per type). Added to `ROADMAP.md`.
- [`String` indexing is a place projection, as the typed tree has it, but a decoded `Char` inside a UTF-8 string has no address] → the backend can implement an `Index` read as a call; a borrow of `s[i]`, if the type checker keeps accepting it, would need a temporary instead. Added to `ROADMAP.md` for `codegen-core`.
- [The slot argument of reference-parameter generators costs a borrow and a `null` temporary at every call that omits such a parameter] → it only exists for reference-typed parameters with defaults, the backend can inline generators of the same file, and it is what keeps the generator's shape independent of its body.
- [Lifetime extension is a rule users must learn] → it only applies to `val r = &temporary`, where the alternative is a borrow error with no workaround but naming the temporary.

## Migration Plan

No external users. `TypedFile` gains a field and `TypeChecker`'s private maps become a `TypeTable`; callers of `check_module` are unaffected. The typed-tree snapshots of fixtures that import types gain their `import` lines and are re-blessed. Rollback is a revert.

## Open Questions

- Integer overflow and division by zero (trap, wrap or check): `codegen-core` decides, and may add an assertion terminator to MIR. Added to `ROADMAP.md`.
- Whether the type checker should reject a default that consumes an earlier parameter (D11) before `borrowck-moves` does. Added to `ROADMAP.md`.
- Whether `prism dump-mir` should accept a directory, like the same open question for `dump-typed-tree`.
