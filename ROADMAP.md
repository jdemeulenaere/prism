# Prism Roadmap & Milestones

Following the **Elephant-Goldfish model** via OpenSpec, this roadmap represents the high-level milestone tracking memory. Each phase is broken down into small, verifiable, test-driven work packets in `openspec/changes/`.

- [`openspec/specs/`](openspec/specs) describes only what is **implemented**.
- [`docs/architecture.md`](docs/architecture.md) describes what is **planned** for the unchecked items below. A change that implements a planned item moves its requirements from there into the specs.

---

## Phase 0: Foundations & Architecture (Completed)
- [x] Project Vision & Invariants established ([`README.md`](README.md), [`openspec/specs/system-invariants`](openspec/specs/system-invariants/spec.md)).
- [x] Compilation & Execution Architecture design ([`docs/architecture.md`](docs/architecture.md) §1–2).
- [x] Zero-Cost Memory Architecture design ([`docs/architecture.md`](docs/architecture.md) §3).
- [x] Package Resolution & Root Discovery Specification ([`openspec/specs/package-resolution`](openspec/specs/package-resolution/spec.md)).
- [x] Syntax & Grammar Specification ([`openspec/specs/syntax`](openspec/specs/syntax/spec.md)).
- [x] AI Pair-Programming Guide ([`AGENTS.md`](AGENTS.md)).
- [x] Initialize Rust workspace (`prism-compiler`):
  - `prism-lexer`: Fast tokenizer.
  - `prism-parser`: Pratt parser + AST generation.
  - `prism-ast`: Abstract Syntax Tree definitions.
  - `prism-abi`: Surface signature extraction & ABI hashing.
  - `prism-cli`: Developer command line interface (`prism`).

---

## Phase 1: Front-End Parser, AST & Package Resolver
- [x] Lexer and Parser for basic language subset:
  - Primitive types, `val`/`var`, basic arithmetic, `fun`, `if`, `when`, `return`, `struct`, `enum`.
  - Default `private` visibility, explicit `package` and `public`.
- [x] Package & import resolver conforming to zero-config, zero-marker directory rules (`crates/prism-resolver`).
- [x] CLI multi-file workspace and package checking (`prism check`).
- [x] Parsing and AST validation test suite.
- [ ] Code formatting command (`prism fmt`).

---

## Phase 2: Type System, Value Ownership & ABI Invalidation (Current)
- [x] Type checker with bidirectional type inference, nominal types, scoping, and diagnostics (`crates/prism-typechecker`).
- [x] ABI surface extraction and deterministic ABI hashing (`crates/prism-abi`, [`compilation-and-abi`](openspec/specs/compilation-and-abi/spec.md)).
- [x] Value ownership transfer, move semantics, borrow checker, and the 4 lifetime elision rules for functions. Delivered by the sequence below, completed by `borrowck-elision`; escaping move-capturing closures remain for `closures`.

### Next up: the road to the borrow checker
Agreed with the user in an `/opsx:explore` session (2026-10-17). Each item is one OpenSpec change, in this order; the decisions behind them are in [`docs/architecture.md`](docs/architecture.md) §1 (pipeline) and §3 (memory model).

1. [x] `statements-and-loops`: a `Statement` AST type (`Block` with an explicit tail expression, assignment as a statement); `while`, `do`/`while`, `for (x in range)` over the built-in `Range`; `break`/`continue` as `Nothing`-typed expressions with `E4015` outside a loop; the `..<` half-open range operator.
2. [x] `value-semantics`: opt-in `: Copy` and `: Clone` markers on structs and enums, checked structurally and recorded in the ABI; the built-in `Copy` and `Clone` tables (`String` is `Clone` but move-only); a built-in `x.clone()`; a move's source must be a whole owned local (`E4017`); top-level `var` removed.
3. [x] `reference-access`: implicit shared borrow at argument sites (temporaries included); implicit copy-out of `Copy` values from borrows, and moving out of a borrow rejected; prefix `*` for writing through a reference (`*r = v`); `var` reference bindings can be re-pointed; passing a `&mut` binding reborrows it. Moving out of a borrow extends `E4017` with the source "borrow" rather than taking a new code, and borrowing a nullable's contents without cloning (`&(T?)` to `(&T)?`) needs a design (both from `value-semantics`).
4. [x] `extension-methods`: methods are extension functions only, declarable in any file, with `fun` / `mut fun` / `consuming fun` receiver modes, implicit `this` and static dispatch; receivers borrow implicitly at the call site. A field takes precedence over an extension of the same name, as a field named `clone` does over the built-in `clone()`, and a user extension named `clone` on a `Clone` type is rejected (from `value-semantics`).
5. [x] `typed-tree`: a `prism-typed-tree` crate produced by the type checker, with resolved bindings, a type on every node, normalized calls and explicit copy/move/borrow adjustments, including the receiver adjustments `extension-methods` decides (`ReceiverAdjustment`); no behavior change.
6. [x] `mir-lowering`: a `prism-mir` crate lowering the typed tree of each file that typechecks to a place-based control-flow graph, with explicit storage, drops and branches, a `prism dump-mir <file.prism>` text format for snapshot tests, and a MIR validator. It decided where temporaries live (temporary scopes, with `val r = &make()` extending its temporary to the binding), how a default value runs (a generator body over borrows of the earlier arguments, with a caller-owned slot for a default that borrows a temporary), and how a `(&mut T)?` lent as `(&T)?` is reborrowed (a tag test and a shared reborrow of the payload).
7. [x] `borrowck-moves`: a `prism-borrowck` crate checking each file's MIR alone, and the `E5xxx` code range; use after move (`E5001`), including conditional and loop moves, consuming receivers and closure captures, reinitialization by assignment, and reads of uninitialized values (`E5002`), with diagnostics showing where the value moved. It also took drop elaboration from step 16: after a file passes, each drop is removed, kept or guarded by a drop flag (`prism dump-mir --elaborated`). By-value receivers are now evaluated before the arguments, and a default value may not move an earlier parameter (`E4017`).
8. [x] `borrowck-exclusivity`: non-lexical borrows from a loan flow and liveness on elaborated MIR, with `E5003` to `E5006` for conflicting borrows and reads, assignments and moves of borrowed values, and values that end while borrowed; disjoint field paths; two-phase receiver borrows by evaluation order (a safe call's receiver is borrowed at its test); call results that keep their input borrows (the caller's half of elision); closures capture by inferred borrow, with the modes recorded in elaborated MIR, may not move a capture (`E4017`), and a capturing closure or function-typed parameter cannot escape (`E5007`). Borrowed temporaries in branch values now live as long as the value, and block-end drops carry the closing brace's span.
9. [x] `borrowck-elision`: elision rules 1–4 derived from signatures alone, for callers (rule 2 now keeps only the receiver borrowed) and callees (`E5008` `DanglingBorrowReturn` for a returned borrow of a local, temporary or by-value parameter, `E5009` `UntiedBorrowReturn` for one derived from an input the rule does not tie the result to); function types elide by the same rules, so a closure whose type takes a borrowed input cannot return its captures; defaults borrow only through shared references; borrows of top-level `val`s are valid everywhere; by-value parameters end at each return; and a `(&mut T)?` lent as a `(&mut T)?` is reborrowed.

### Then: the road to Milestone 2
Planned autonomously on 2026-10-17, when the user asked for iteration up to Milestone 2 (see [`docs/decisions-for-review.md`](docs/decisions-for-review.md)). Each item is one OpenSpec change, in this order; the order and scope may be revised as changes land, and any revision is logged there.

10. [x] `generics`: type parameters on functions, extensions, structs and enums (`fun <T> id`, `fun <T> Box<T>.get()`), checked once at declaration with `Copy`/`Clone` bounds; explicit (`f<Int>(x)`) and inferred type arguments; borrow-free type arguments; conditional markers on generic types; polymorphic typed trees and MIR, and generic signatures in the ABI. Needed for `State<T>`. Monomorphization was split out into its own change, item 17 (2026-10-18). Also: one extension per receiver constructor (`E4010`), the new codes `E4020` to `E4022`, generic default value generators, MIR aggregates with type arguments, and expected types reaching `?:` fallbacks and borrowed arguments (from the review).
11. [ ] `compound-assignment`: `+=`, `-=`, `*=`, `/=`, `%=` as statements.
12. [ ] `abi-cache`: decide the ABI hash algorithm, write `.abi` and artifact caches, and verify compilation avoidance (body edits recompile no importer).
13. [ ] `prism-fmt`: the `prism fmt` code formatter.
14. [ ] `codegen-core` (Phase 3): a `prism-codegen` crate lowering MIR to Cranelift; `prism run` for `Int`/`Bool`/`Double`, functions, control flow and loops; a `print` built-in; an executable-fixture runner comparing stdout. Generic bodies wait for `monomorphization`.
15. [ ] `codegen-aggregates`: structs, enums, nullables, references and `when` in the backend.
16. [ ] `runtime-strings-and-drops`: owned `String`, drop glue (what dropping a `String`, a struct or a `Heap<T>` does, including a destructor dropping owned fields and heap allocations at scope exit), `clone()`, and `Heap<T>` construction. Drop elaboration on MIR was done by `borrowck-moves`.
17. [ ] `monomorphization`: instantiating generic MIR per type arguments (from `generics`): substitution, type-parameter drops resolved to drop glue or removed, flattened `T?` rewrites, rejecting polymorphic recursion, and cross-file instantiation, shipping each generic's elaborated polymorphic MIR, deciding where instances live and what the Monomorphization Hash covers, with re-monomorphization only where a generic body changed ([`docs/architecture.md`](docs/architecture.md) §1).
18. [ ] `closures`: closure conversion in the backend, then escaping move-capturing closures (needed for `onClick`).
19. [ ] `hot-reload`: function-pointer tables in Tier 1 and `prism run --watch` patching changed functions while the program runs.
20. [ ] `compose-runtime` (Phase 4): slot table, group keys and memoization.
21. [ ] `compose-transform`: the compiler transform of `compose fun` (composer parameter, group keys, change-tracking bitmasks, skipping).
22. [ ] `state-handles`: `remember`, `mutableStateOf` and `Copy` `State<T>` handles with generational indices; invalidation and recomposition.
23. [ ] `ui-toolkit`: a UI node tree with layout (`Column`, `Row`, `Text`, `Button`), a desktop renderer and a headless renderer for tests, and event dispatch.
24. [ ] `compose-hot-reload`: hot reload of composable bodies that keeps slot-table state.
25. [ ] `reactive-app-verification`: real reactive example apps (counter, todo list) running natively, with tests that no allocation outlives its owner.

Deliberately left until after the borrow checker (from the same session):
- [ ] `Heap<T>` construction and unboxing, including whether its contents can be moved out, whether `*h = v` writes a `Heap`'s contents, and whether `Heap<U>` to `Heap<T>` stays covariant, which changes the representation the way reference covariance did (from `reference-access`), and whether a `consuming` call may move its receiver out of a `Heap<T>`, `E4017` today (from `extension-methods`).
- [ ] Payload-binding `when` patterns and destructuring declarations (`val (name, email) = user`), the way to take ownership of a struct's non-`Copy` fields. A `consuming fun` needs it to take `this` apart (`Account(name)` is `E4017` today, from `extension-methods`).
- [ ] Escaping closures that capture by move, needed by compose event handlers such as `onClick`.
- [ ] Trait conformance syntax: with extension-only methods, conformances need their own block (Swift's `extension T : P`, Rust's `impl P for T`).
- [ ] Field visibility (`private var balance`), the encapsulation boundary for extension functions. The `E4010` rule for an extension named like a field then applies only to fields visible where the extension is declared (from `extension-methods`).
- [ ] Static and companion functions (`User.create()`) (non-goal of `extension-methods`).
- [ ] Loop labels (`break@outer`) (deferred by `statements-and-loops`).
- [ ] Deferred `val` initialization (`val x: Int` assigned once on every path).
- [ ] Iterating anything other than `Range` in `for` loops, which needs generics and an iteration trait (deferred by `statements-and-loops`).
- [ ] Decide whether `while (true)` with no `break` should have type `Nothing`, so a non-`Unit` function needs no `return` after it; this would only relax the current rule (non-goal of `statements-and-loops`).
- [ ] Decide whether an unbraced loop or `if` body may itself be a loop (`for (i in a) for (j in b) g(i, j)`, valid in Kotlin); today an unbraced body is an expression or an assignment only (from `statements-and-loops`).
- [ ] Possible relaxation: partial moves out of owned locals, if destructuring proves insufficient.
- [ ] Possible relaxation: moving a field out of a temporary (`make().name`), rejected by `value-semantics` so that no temporary needs a partial drop.
- [x] Moves of variables captured by closures: `value-semantics` treats a capture as a local; with inferred-borrow captures (`borrowck-exclusivity`) such a move is a move out of a borrow. Until then `run { take(s) }; take(s)` passes `prism check`, and a closure that moves a capture leaves the enclosing body to drop it again, as does a closure in a default value moving an earlier parameter (`borrowck-moves`, D11 and D13). Decided and done in `borrowck-exclusivity`: it is `E4017` in the type checker.
- [ ] A syntax for reborrowing a `&mut` into a local (`val r2 = …`): prefix `*` is writes-only, so today a `&mut` can only be reborrowed by passing it as an argument (from `reference-access`).
- [ ] Possible relaxation: copy-out in `if`/`when`/`?:` joins (`if (c) r else 5`, `first() ?: 0` with a `&Int` side), rejected for now because the join's type would depend on which side is a reference (from `reference-access`).
- [ ] Possible relaxation: distributing a borrow through branches and `?:` to their leaves, so `greet(u.nick ?: "anon")` borrows `u.nick` instead of moving it; rejected for now in favour of "a borrow points at one place or one temporary" (from `reference-access`).
- [ ] Smart casts (`if (n != null) greet(n)`), and borrowing through a safe navigation (`&u?.nick`) as an optional place rather than a temporary (from `reference-access`). `?.` receivers already are optional places (`extension-methods`, D10); extend them to explicit `&u?.nick` and to `(&T)?` arguments.
- [x] How long an explicitly borrowed temporary in a `val` initializer lives (`val r = &make()`), for `borrowck-exclusivity` or `borrowck-elision` (from `reference-access`). Decided and done in `mir-lowering`: the temporary lives as long as the binding.
- [ ] Extensions on nullable receiver types (`fun String?.orEmpty()`), and which extension a call on a `T?` picks when both `T` and `T?` have one (non-goal of `extension-methods`).
- [x] Generic receivers (`fun <T> Heap<T>.size()`), with `generics`; today a receiver is a single type name (non-goal of `extension-methods`). Done in `generics` for nominal receivers (`fun <T> Box<T>.get()`, `fun Box<Int>.sum()`); `Heap<...>` receivers stay `E2001` because calls look through `Heap` layers, and are tracked below.
- [ ] Extension properties (`val User.fullName: String`) (non-goal of `extension-methods`).
- [ ] Bounds on struct and enum type parameters (`struct Box<T : Copy>`), `E2001` today; they need a well-formedness check at every annotation (non-goal of `generics`, D5).
- [ ] `==`, `!=` and orderings on type parameters, with traits and an equality bound; `E4002` today (from `generics`, D5).
- [ ] Explicit type arguments without a call (`Option<Int>.None`, `id<Int>` as a value), and generic functions as values with an expected function type (`val f: (Int) -> Int = id`), `E4013` today (from `generics`, D8).
- [ ] Extensions on a bare type parameter (`fun <T> T.also()`) and on `Heap<...>` receivers, `E2001` today (non-goal of `generics`, D10).
- [ ] Possible relaxation: an opt-in bound for borrowed type arguments (`Box<&T>`), which `E4022` rejects today (from `generics`, D7).
- [ ] Possible relaxation: global or later-use type argument inference (Kotlin's constraint solving), so `take(Box(null), 1)` for `fun <T> take(b: Box<T>, c: T)` and an empty `listOf()` fixed by a later use are accepted; local inference rejects them today (risk of `generics`, D8, confirmed in its review).
- [ ] Possible relaxation: conditional markers computed from the field types' own markers rather than syntactically, so `struct W<T> : Copy { val h: Handle<T> }` is `Copy` for every `T` when `Handle<T>` is (from `generics`, D6).
- [ ] Trailing commas in call type-argument lists (`id<Int,>(1)`), which annotations and type parameter lists accept but the call rule does not (found while reviewing `generics`).
- [ ] Decide whether the typed-tree dump should distinguish inferred type arguments from written ones (open question of `generics`).
- [ ] Decide whether the `E4013` help for an uninferable generic call should print the full suggested call (`make<...>()`) or only the type parameter's name (open question of `generics`).
- [ ] Method references and extensions as values (`u::greet`); reading `u.greet` without a call is `E4006` today (non-goal of `extension-methods`).
- [ ] Decide whether `this` in a `consuming fun` should be mutable, like Swift's consuming parameters; today it is a `val` and is moved into a `var` first (from `extension-methods`, D4).
- [ ] Decide whether a bare `clone()` in an extension body calls the built-in `clone()` of a `Clone` receiver; implicit `this` covers only fields and extensions, so it is `E4001` today (found while archiving `extension-methods`).
- [ ] Representation-changing subtyping: function-type variance through nullable parameters (`(Int?) -> Unit` given for `(Int) -> Unit`) and `Heap<Int>` given for `Heap<Int?>` are `Subtype` coercions in the typed tree that change layout. `codegen-core` must generate adapters for them, or the type system must drop those subtyping rules (risk from `typed-tree`, D6; see also the `Heap<T>` covariance item above).
- [ ] Decide whether the discarded branch of an `if` without `else` in a consuming position is consumed: `val u = if (c) s` moves `s` today, because branches of a consuming position consume, although the branch's value is discarded (found while reviewing `typed-tree`).
- [ ] Integer overflow and division by zero: trap, wrap or check, possibly with an assertion terminator in MIR; `codegen-core` decides (open question of `mir-lowering`).
- [x] Decide whether the type checker should reject a default value that consumes an earlier parameter (`fun f(a: String, b: String = a)`); in its generator it is a move out of a borrow, which `borrowck-moves` must reject otherwise (open question of `mir-lowering`, D11). Decided and done in `borrowck-moves`: it is `E4017`.
- [ ] Layouts and drop glue of imported types whose contents are not visible from the direct imports' ABIs (a struct of a directly imported module holding a type of a module the file does not import): the ABI could export a "needs drop" bit and a layout, or symbolic drop-glue functions per type, for `codegen-aggregates` and `runtime-strings-and-drops` (risk from `mir-lowering`).
- [ ] `String` indexing and `Char`: `s[i]` is a place projection in the typed tree and MIR, but a `Char` decoded from UTF-8 has no address, so the backend can implement a read as a call while a borrow of `s[i]` would need a temporary; decide indexing semantics (byte or character) for `codegen-core` (risk from `mir-lowering`).
- [x] `borrowck-moves` must reject reading, in a `do`/`while` condition, a body local whose declaration a `continue` skipped: MIR marks it live without a value on that path (found while reviewing `mir-lowering`, N5). Done: `E5002`, labelled at the `continue`.
- [x] Decide whether a by-value receiver (`consuming fun`, or a `Copy` receiver) should be read before the arguments, as Rust does, rather than after them as a borrowed receiver is: today `s.consume(f(&mut s))` moves `s` after `f` mutates it (found while reviewing `mir-lowering`, D10). Decided and done in `borrowck-moves`: it is moved or copied first, so that call is a use after move.
- [ ] Possible relaxation: lifetime extension through assignments (`r = &make()`) and through a borrowed argument whose call result outlives the statement (`val r = first(&make(), &s)`), both `E5006` today (from `mir-lowering`, D7; extension through branches was done by `borrowck-exclusivity`, D20).
- [ ] Decide whether `prism check` should report the borrow and type errors of every file instead of stopping at the first file with errors, which the LSP will want (open question of `borrowck-moves`).
- [ ] Decide whether a move's label should name why the value moves ("moved by the `consuming` receiver of `toAccount`"), which needs MIR to record the reason (open question of `borrowck-moves`).
- [ ] Decide whether move paths and the dataflow framework should become their own crate once the backend needs them (open question of `borrowck-moves`).
- [x] Record which operand of a MIR `Call` is the receiver, which elision rule 2 needs (`borrowck-elision`, from `borrowck-moves`, D16). Decided and done in `borrowck-elision`: the callee's declaration path names an extension's receiver, so MIR needs no new field.
- [ ] With smart casts, accept a nullable used after its payload moved on the paths where it is `null`: `u?.toAccount(); if (u == null) ...` is a use of a possibly moved value today, because moving a payload counts as moving the nullable (from `borrowck-moves`, D3).
- [ ] Deferred `val` initialization will reuse `E5002` for a read before the assignment, with a label for the declaration without a value (from `borrowck-moves`, D7).
- [ ] `mk()?.name?.clone()` is `E4017` ("cannot move out of field 'name' of 'mk()'") although `u?.name?.clone()` on a place is accepted; decide whether cloning through a safe navigation on a temporary is a move (found while reviewing `borrowck-moves`; the behaviour predates it).
- [x] `borrowck-elision` must make true on the callee's side what `borrowck-exclusivity` assumes at call sites (D9): a function's or closure's returned borrow derives only from the inputs its elision rule allows, so rule 2 can be narrowed at call sites once MIR records the receiver operand; a default value's generator never returns a borrow of a by-value earlier argument; a returned borrow never derives from a function-typed argument's result (`fun call(h: () -> &Int): &Int = h()`), which a caller does not count as an input borrow; and decide whether a closure may return borrows of its captures, and whether a body may store an input borrow through another input (`*slot = v` for `slot: &mut &Int, v: &Int`). Done in `borrowck-elision`: the return check enforces each rule; a closure returns borrows of its captures only when its type takes no borrowed input; storing an input borrow through another input is accepted.
- [ ] Possible relaxation: per-field closure captures (Rust 2021's precise captures), so `{ u.name = "z" }` borrows `u.name` rather than all of `u` (from `borrowck-exclusivity`, D8).
- [ ] Possible relaxation, with `closures`: accept a capturing closure written through a reference into the body's own storage (`*p = { x }` where `p` borrows a local), stored in a field of a local struct, or used as a default value, all `E5007` today (from `borrowck-exclusivity`, D12 and its open question).
- [ ] Revisit liveness ignoring drops if move-capturing closures (`closures`) can also borrow: a loan carrier would then own a value with a destructor (risk from `borrowck-exclusivity`, D5).
- [ ] Decide whether a read while a value is mutably borrowed deserves its own code rather than `E5003` (open question of `borrowck-exclusivity`).
- [ ] Possible relaxation: let a safe call's arguments read the tested nullable under a mutable receiver (`n?.push(n?.size() ?: 0)`, `E5003` today) with a shallow fake borrow of the tested value until the receiver is borrowed (from `borrowck-exclusivity`, review of D7).
- [ ] Revisit the loan flow's dense bit set over loan-carrying locals times loans if bodies grow large, with a sparse set per local (risk from `borrowck-exclusivity`).
- [x] A `mut fun` whose reference default borrows `this` (`mut fun User.rename(n: &String = name)`) cannot be called without that argument: `u.rename()` is `E5005`, "cannot move out of a temporary value", at the receiver. Name the receiver in that diagnostic, or reject such a default at its declaration (found while reviewing `borrowck-exclusivity`). Done in `borrowck-elision`: the default is `E5009` at its declaration.
- [x] An `E5006` for a borrow written through a `&mut` argument labels its later use at the function's signature; label the parameter instead (found while reviewing `borrowck-exclusivity`). Done in `borrowck-elision`, together with labelling an ending parameter at its declaration.
- [x] Lending a `(&mut T)?` value where a `(&mut T)?` is expected (`val m: (&mut String)? = &mut p.nick; edit(m)`, or forwarding a `(&mut T)?` parameter) crashes MIR lowering ("a `(&mut String)?` place read in place is used as a value"): decide whether it moves the value or reborrows it, as lending it as a `(&T)?` does (found while reviewing `borrowck-exclusivity`; the crash predates it, from `mir-lowering`). Decided and done in `borrowck-elision`: it is reborrowed.
- [ ] Possible relaxation: promote constant temporaries to static storage, so that `val R: &Int = &1` and `fun f(): &Int = &1` are accepted rather than `E5008`; this needs constant evaluation (open question of `borrowck-elision`, D7).
- [ ] Possible relaxation: extend a default value's slot to the enclosing block when its call initializes a `val`, so `val h = hello(&u)` for `fun hello(u: &User, n: &String = u.name): &String` is accepted rather than `E5006`; this changes the `mir` slot rule (open question of `borrowck-elision`, D10).
- [ ] Possible relaxation: let a closure whose type takes a borrowed input return borrows of its captures (`app { k -> &n }`, `E5009` today) where the closure is called in the body that builds it; tying a function type's result by its signature alone forbids it everywhere (risk from `borrowck-elision`, D8).
- [ ] A borrow of a loop's local assigned to a reference that is then returned (`r = &n` in a loop, then `return r`) reports both `E5006` and `E5008` for one mistake; report only the `E5008` (found while reviewing `borrowck-elision`).

### Other Phase 2 items
- [ ] Caching or serializing typed trees, interning types (nodes keep their `Type` behind `Body::type_of` so this stays possible) and incremental re-checking of bodies (non-goals of `typed-tree`).
- [ ] Whether `prism dump-typed-tree` should also accept a directory, dumping every file of a package (open question of `typed-tree`).
- [ ] Caching or serializing MIR, and MIR optimizations such as constant folding (non-goals of `mir-lowering`). Whether `prism dump-mir` should also accept a directory (open question of `mir-lowering`).
- [ ] Decide the authoritative ABI hash algorithm: BLAKE3 (original design) or the current FNV-1a ([`docs/architecture.md`](docs/architecture.md) §1).
- [ ] `.abi` cache and compilation avoidance ([`docs/architecture.md`](docs/architecture.md) §1).
- [ ] Incremental compilation verification: ensuring body edits produce zero downstream recompilations.
- [ ] Compound assignment (`+=`, `-=`) and bitwise operators (deferred by the `lexer-parser` change).
- [x] Generics: type parameter checking and monomorphization (deferred by `type-checker-core`). Until `Copy` and `Clone` bounds exist, type parameters are neither `Copy` nor `Clone` (from `value-semantics`). Checking, `Copy`/`Clone` bounds and polymorphic MIR are done in `generics`; monomorphization is its own item, step 17 of the road to Milestone 2.
- [ ] Reject unknown type names in annotations (`E4001`). This needs a builtin type list, including sized integers like `UInt64`, plus generic scoping; until then undeclared names are accepted as opaque nominal types (deferred by `resolve-nominal-type-annotations`). Generic scoping now exists (`generics`), so only the builtin list and the migration of the opaque-name fixtures remain.
- [ ] Qualified type paths such as `models.User`; imports currently bring symbols in by bare name (deferred by `resolve-nominal-type-annotations`).
- [ ] Fix: calling a struct-typed value (`p()` where `p: Point`) goes through the constructor branch of `infer_call_expr` (noted in `resolve-nominal-type-annotations`).
- [ ] Decide whether safe-navigation assignment (`a?.b = c`, allowed in Kotlin) is valid; it is currently rejected (from the parser plain fixes, `ecc5da3`).
- [ ] Revisit the parser's 256-level nesting limit (E2002) before the LSP: a debug build needs about 2.75 MB of stack at the limit, more than a default 2 MB worker thread (from `8f1f932`).
- [ ] Implicit `it` parameter for single-parameter closures (deferred by `harden-parser-grammar`).
- [ ] Lint for `fun f() = { … }`, which returns a closure as in Kotlin (follow-up from `harden-parser-grammar`).
- [ ] Unify the missing-return-type rule: `prism-abi` accepts a block-bodied exported function without a return type as `Unit` and only rejects expression bodies (`E3001`), while the type checker rejects both (`E4011`). Tighten the ABI rule or relax the specs (open question in `resolve-nominal-type-annotations`).
- [ ] Reject recursive struct and enum layouts that lack `Heap<T>` indirection (deferred by `fix-cli-workspace-audit`).
- [ ] Parser error recovery: report several syntax errors per file instead of stopping at the first, needed for the LSP (deferred by `lexer-parser`, and again by `harden-parser-grammar` until the AST has error nodes).
- [ ] Traits or interfaces for polymorphism and dynamic dispatch; not designed yet (deferred by `type-checker-core`). Custom `clone()` bodies wait for them: today a `Clone` type's clone clones each field (from `value-semantics`).
- [x] Reject borrows in enum payloads (`enum A { X(&Int) }`), as the parser already does for struct fields; today such an enum may even be `: Copy` (found by `value-semantics`). Done as a plain fix, which also rejects borrows nested in `?` or a type argument.
- [x] Decide whether a top-level declaration may shadow an imported symbol instead of reporting `E4010` (follow-up from `typechecker-soundness`, D7). Decided and done in `shadowing-and-import-paths`.
- [x] Decide whether a local declared in a function body may shadow a parameter (`fun f(a: Int) { val a = 2 }`, accepted today; Kotlin warns, Rust allows). Fixture `type-system/duplicate-symbol-declaration-rejected--local-duplicating-a-parameter` pins current behavior. Decided and done in `shadowing-and-import-paths`.
- [x] Decide which stage owns `.`/`..` import segments: the parser rejects them first (`E2001`), so the spec'd resolver code `E1006` is only reachable for `import self`. The `package-resolution/import-attempting-directory-traversal--*` fixtures pin current behavior. Decided and done in `shadowing-and-import-paths`.
- [x] Report `E4014`, with the import to add, when an annotation names a type known only through an import's signature; such a name is currently an opaque type, so its first member access reports `E4006` (follow-up from `typechecker-soundness`, D9). Decided and done in `shadowing-and-import-paths`.
- [ ] A warnings channel for diagnostics that are not errors, such as `?:` on a non-nullable left side; Prism reports only errors today (non-goal of `typechecker-soundness`).

---

## Phase 3: Tier 1 Native Execution & Hot Reload (Cranelift JIT)
- [ ] Intermediate Representation (Prism IR / SSA).
- [ ] Cranelift JIT native code generator ([`docs/architecture.md`](docs/architecture.md) §2, §4).
- [ ] In-memory native execution via `prism run file.prism` (<15ms startup).
- [ ] Hot-reload runtime prototype: dynamic function-pointer patching with state preservation.
- [ ] Integration test suite running executable `.prism` native test fixtures.

---

## Phase 4: First-Class Compose Engine & UI Stack (Milestone 2)
- [ ] Compose runtime library: Slot table, group keys, memoization engine.
- [ ] Compiler `compose fun` transform: synthetic change-tracking bitmasks and slot injection.
- [ ] State handle system (`remember { mutableStateOf(...) }`).
- [ ] Declarative UI prototype with hot-reload rendering (desktop/mobile UI application).
- [ ] Verification of real-world reactive UI apps running natively with zero GC pauses.

---

## Phase 5: Tier 2 Release Backend (LLVM AOT), LSP & Self-Hosting (Milestone 3)
- [ ] Tier 2 LLVM AOT native code generator (`prism build --release`) with whole-program optimization and LTO.
- [ ] Prism Language Server Protocol (LSP) for VS Code / IDEs.
- [ ] Port bootstrap Rust compiler to native Prism.
- [ ] Full self-hosting compiler compilation and bootstrap verification.
