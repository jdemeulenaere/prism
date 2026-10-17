# Design: borrowck-exclusivity

## Context

See `proposal.md` for motivation. This change is step 8 of "the road to the borrow checker" in `ROADMAP.md`, agreed with the user in the `/opsx:explore` session of 2026-10-17: non-lexical borrows, disjoint field paths, two-phase receiver borrows; closures capture by inferred borrow and a capturing closure cannot escape. The planned requirement is `docs/architecture.md` §3, "Stack-Scoped Borrowing and Mutation Rules", with seven scenarios: concurrent shared borrows; an exclusive `&mut` prevents any other borrow or access through the owner, and assigning or moving a borrowed value is rejected (`E5xxx`); a borrow ends at its last use (`var x = 1; val r = &x; print(r); x = 2` accepted); disjoint field borrows; two-phase receiver borrows (`v.push(v.size())`); closures capture by inferred borrow, held while the closure value is live; capturing closures (and function-typed parameters) do not escape, while a closure without captures may. The design notes there say: non-lexical borrows come from liveness on the MIR, and closures stay conservative until escaping closures are designed.

`ROADMAP.md` assigns this change three follow-ups:

- "Moves of variables captured by closures": `run { take(s) }; take(s)` passes `prism check`, a closure that moves a capture leaves the enclosing body to drop it again, and so does a closure in a default value moving an earlier parameter (`borrowck-moves`, D11 and D13);
- "How long an explicitly borrowed temporary in a `val` initializer lives": already settled and ticked by `mir-lowering` (the temporary lives as long as the binding), so nothing is left but to check the forms that are not extended (D19);
- from `borrowck-moves` D16: run the exclusivity check on the elaborated MIR, and treat the drops that remain as accesses.

The user was not available while this change was proposed, so every question beyond the agreed scope is decided here, marked **(autonomous)**, and logged in `docs/decisions-for-review.md`.

The consumers, each named where a decision serves it: `borrowck-elision` (step 9: the callee side of the elision rules, `DanglingBorrowReturn`, borrows of top-level `val`s), `closures` (step 17: closure conversion and escaping move-capturing closures, which read the capture modes this change records), and the backend, which relies on MIR that never aliases a `&mut`.

Current state:

- **MIR** (`crates/prism-mir`, the `mir` spec). Every borrow is an `Rvalue::Borrow { mutable, origin, place }` (origins `Written`, `Argument`, `Receiver`, `Conversion`, `Operator`, `Default`). Shared references are `Copy` (`_3 = copy _2`), `&mut` ones are moved or reborrowed at argument sites (`_2 = &mut (*_1) (argument)`). A mutable receiver's place is evaluated first and its borrow taken after the arguments (`_3 = &_1 (receiver)`; `_2 = call size(move _3)`; `_4 = &mut _1 (receiver)`; `call push(move _4, move _2)`). A default's generator takes shared borrows (`Default` origin) of the earlier arguments' temporaries and a `&mut R?` slot. A `(&T)?` view is a tag test, a `Conversion` borrow of the payload and a `WrapNullable`. A closure value is `Rvalue::Closure { body, captures: Vec<Place> }`; its body has capture locals that stand for the captured bindings, and a move of one is lowered as `move _1` (the bug of the follow-up). Every operand carries its span. Scope exits (drops, storage ends) carry their scope's span, which for a block is the span of the whole block.
- **Borrow checker** (`crates/prism-borrowck`). `check_file` gathers `MoveData` per body, solves `MaybeInitialized` and `ReachingUninitializations` over `dataflow.rs` (forward and backward, with a `ResultsCursor` and an edge hook), checks uses (`check.rs`) and elaborates drops (`elaborate.rs`), returning `MirPhase::Elaborated` MIR or `BorrowError`s (`E5001`, `E5002`), each with a primary `Label`, related labels, notes and helps. `dominators.rs` finds back edges. `verify_elaborated` re-checks elaboration in the CLI's tests.
- **Type checker.** `check_move_source` rejects moves out of fields, borrows, top-level `val`s and, inside a default value, of an earlier parameter (`is_default_input`). The builder (`builder.rs`) records each closure's captures as the bindings older than the closure (`first_local`) that its body uses.
- **Syntax.** `*` is writes-only, so `&mut *r` does not parse: a `&mut` is reborrowed only at argument sites and by mutable receivers through references, and a borrow of a field through a reference (`&mut m.name` for `m: &mut User`) is a reborrow of part of its referent.

## Goals / Non-Goals

**Goals:**
- Every borrow is respected for as long as the reference holding it can still be used, and no longer: a conflicting access, an assignment or move of a borrowed value, and a value that ends while borrowed are rejected, each with labels saying where the borrow was made and where it is used later.
- The analysis reads nothing but the file's MIR, and a call is analysed from its MIR operands and parameter types alone, so borrow checking keeps the single-file compilation and deterministic hermetic builds invariants, and the elision rules of step 9 need no new ABI data.
- Closures are sound with no new syntax: inferred-borrow captures, no escape of capturing closures, no move of a capture.
- A precise boundary with `borrowck-elision`, so that step only adds the callee's side.

**Non-Goals:**
- The callee's side of elision: checking what a function, closure or default returns, `DanglingBorrowReturn`, the receiver-only rule 2 at call sites and recording a call's receiver operand, borrows of top-level `val`s, and storing an input borrow through another one (`borrowck-elision`, see D9).
- Escaping, move-capturing closures and closure conversion (`closures`); per-field (precise) captures.
- A syntax for reborrowing a `&mut` into a local, lifetime extension through assignments, and partial moves.
- Reporting the errors of every file per run (unchanged, see `borrowck-moves`'s open question).

## Decisions

Decisions marked **(user)** were agreed in the exploration session, and **(autonomous)** ones were made while proposing this change and are open to review. The others follow from these or from existing rules.

### D1. The rules extend `references`; the capture-move rule extends `ownership` (autonomous)

The `references` spec's purpose says borrow conflicts and returned-borrow lifetimes "belong to the borrow checker and are added here when it exists", as `ownership` said of use after move (D2 of `borrowck-moves`). So the conflict, liveness, call, closure and escape rules join `references`; moving a capture is a move-source rule and joins `ownership` next to "A Default Value Does Not Move Earlier Parameters"; capture modes and block-end spans join `mir`; the new codes go to the `diagnostics` catalog. Rejected: a `borrow-checking` capability, for the reason D2 of `borrowck-moves` gives.

### D2. The loan model: loans flowing into locals, live where a local holding them is live (autonomous)

A **loan** is one borrow made in a body: its place, whether it is mutable, its location and span, and its origin (D3). The model has two analyses over `dataflow.rs`:

- **Liveness** (backward, bits: loan-carrying locals, D5): a local is live at a point when some path from it reaches a use of the local before the local is wholly reassigned.
- **Loan flow** (forward, bits: pairs of a loan-carrying local and a loan, D4): which loans each local may hold at each point. A borrow puts its loan in its destination; copies, moves, wraps, reborrows and call results pass loans on; a whole-local assignment replaces what the local holds.

A loan is **live** at a point when a local that is live there may hold it there. An access conflicts with every live loan of an overlapping place (D6). This is the location-sensitive formulation of Polonius ("which loans can reach a live local"), computed with the same bit-set dataflow as `borrowck-moves`, without region variables or outlives constraints.

Why this is enough, and what it buys:

- **Non-lexical borrows come from liveness alone.** In `var x = 1; val r = &x; print(r); x = 2`, `r` is not live after `print(r)`, so no live local holds the loan at `x = 2`.
- **It is more precise than Rust's NLL where Prism needs it**, because loans are tracked per point rather than per region: `var r: &Int = &a; r = &b; a = 3; print(r)` is accepted (NLL rejects it, since `r`'s single region contains the point of `a = 3`), which is exactly the re-pointing of `var` references that `reference-access` made a feature.
- **Elision needs no region names.** With no lifetime annotations (the zero-cost memory-safety invariant), a signature never relates regions explicitly: the elision rules say only which inputs a returned borrow comes from. At a call site that is "the result holds the loans of those inputs" (D9), and on the callee's side "what the return place holds at each return derives only from those inputs", which `borrowck-elision` checks with the same loan flow by giving each input borrow an origin fact at entry (D21). NLL's universal regions and outlives constraints exist to check named lifetimes in signatures, which Prism does not have.
- **Closures fit the same model**: a closure value is a local holding the loans of its captures (D10).

Rejected: NLL-style regions (sets of points per reference, subset constraints from assignments, liveness constraints, then a "borrows in scope" dataflow), which needs a constraint solver and rejects re-pointing; full Polonius (Datalog over origins, subset relations per point and loan invalidations), whose generality buys nothing without lifetime parameters; lexical borrows, which the user rejected.

### D3. What a loan is, and which locals carry loans (autonomous)

Loans are created by:

- every `Rvalue::Borrow { mutable, origin, place }`, whatever its origin;
- every capture of an `Rvalue::Closure`, shared or mutable by the inferred mode of D10.

A type **carries loans** when it contains a reference or a function type outside a nominal type: `&T`, `&mut T`, a function type, and nullables of these (`(&T)?`, `(() -> Int)?`). Structs and enums never carry loans, since their fields and payloads cannot be borrows (the `syntax` spec) and a capturing closure cannot be stored in them (D12). Only locals of such types take part in liveness and loan flow, which keeps both analyses small.

Besides loans, each argument and capture local of a type containing a function type holds one **function input** fact at entry: an opaque stand-in for the loans of a closure passed in from outside, which never conflicts with an access but makes the escape check of D12 reject a function-typed parameter that escapes. `borrowck-elision` adds the same kind of fact for reference-typed inputs (D21).

### D4. Loan flow: how loans move between locals (autonomous)

The state is, for each loan-carrying local, the set of loans (and function inputs) it may hold. At entry, it is the function inputs of D3. Effects, in each statement's evaluation order (D6):

| MIR | Effect on what locals hold |
|---|---|
| `l = &p` or `l = &mut p` (loan L) | `l` holds L and every loan held by the root local of `p`: a reborrow `&mut (*r)`, a field through a reference `&mut (*m).name` and a borrow of a reference `&r` all keep `r`'s loans alive while `l` is live |
| `l = copy p` / `move p`, `wrap`, `subtype` of an operand | `l` holds the loans of the operand's root local (`copy (*rr)` of `rr: &&Int` gives the inner reference, conservatively all of `rr`'s loans) |
| `l = closure [captures]` | `l` holds each capture's loan and the loans of each captured root local; each loan-carrying variable the closure captures mutably also receives every loan `l` holds, since calling the closure may write any of them into it |
| `l = call f(args)` | `l` holds the loans of the call (D9); each loan-carrying local a call may write (the roots of mutable loans held by a `&mut` argument whose referent carries loans, or by the callee value) receives every loan of the call's input borrows |
| a whole-local assignment or call destination `l` | first removes everything `l` held (re-pointing), then adds the above; and kills every loan whose place goes through a dereference of `l` (`(*l)…`), which no longer names what that loan borrowed: whoever holds such a loan also holds `l`'s old loans, which keep the old referent borrowed |
| a write through a dereference `(*r)… = v` | `r` and the root locals of the loans `r` holds receive `v`'s loans (`*rr = &a` makes the `r` that `rr` borrows hold `a`'s loan) |
| `move l`, `StorageLive(l)`, `StorageDead(l)` | `l` holds nothing |

Loans of places rooted at a local are never added to that local itself: a reference cannot point into itself, so `set(&mut r, &b)` or a closure that captures `r` mutably gives `r` the loan of `b` but not the loan of `r` it was passed, which would otherwise make every later read of `r` a conflict.

`Discriminant`, operators, aggregates and constants produce values that carry no loans (a top-level `val`'s constant `&T` holds none). The move-only and `Copy` distinctions do not matter here: a moved reference is never used again (`E5001`), and a copied one shares its loans.

References of references and reborrows need no special case: the loans of the inner reference travel with every borrow and copy built from it, so `x` stays borrowed while `rr` in `var r = &x; val rr = &r` is live, and `m`'s loan of `u` stays live while `n` in `val m = &mut u; val n = &mut m.name` is live. Nullable views are the same: `val a: (&String)? = &p.nick` is a tag test, a `Conversion` borrow of the payload and a `wrap`, so `a` holds the loan of `p.nick`.

Rejected: tracking loans stored inside structs (they cannot hold any); ignoring writes through references (`*rr = &a` would leave `a` unprotected); killing loans by assignment only in NLL's way (it would still need regions).

### D5. Liveness: uses and the places that count as live at a return (autonomous)

A loan-carrying local is **used** by any operand, borrow, read, discriminant, call of a function value or closure capture whose place is rooted at it, including a write through `*l`; it is **defined** (killed) by a whole-local assignment or call destination, and by `StorageDead`. Two choices:

- **Drops are not uses.** Only closure values among loan carriers need a drop, and a closure's environment holds only borrows, so dropping it reads nothing it borrowed; making the drop a use would keep every closure's borrows until the end of its block, which the scenario "held while the closure value is live" rules out. Move-capturing closures (`closures`) own values instead of borrowing them, so they do not change this.
- **The return place is not used by `Return`.** What a body returns is the callee's side of elision (D9), so `return &x` is not reported here (it is `borrowck-elision`'s `DanglingBorrowReturn`). But a closure's capture locals, and arguments of type `&mut R` whose `R` carries loans, are **used at every return**, because the caller sees them afterwards: a closure that runs `r = &n` for its own local `n`, or a function that runs `*slot = &n`, is then rejected when `n` ends (`E5006`, D6).

### D6. Accesses and conflicts (autonomous for the codes and the evaluation-order rule; the conflicts themselves user)

Each effect of a location is an **access** to a place, checked against the loans live there:

| MIR | Access | Conflicts with live loans of an overlapping place that are | Code |
|---|---|---|---|
| `copy p`, `Read(p)`, `Discriminant(p)`, a switch operand, a function-value callee, an index local | read | mutable | `E5003` |
| `&p`, a shared capture | shared borrow | mutable | `E5003` |
| `&mut p`, a mutable capture | mutable borrow | any | `E5003` |
| `move p` | move | any | `E5005` |
| assignment or call destination `p` | write | any, except loans of places reached from `p` through a dereference of a reference | `E5004` |
| `drop p` | drop | any, except through a dereference of a reference | `E5006` |
| `StorageDead(l)` | end of storage | any loan of a place rooted at `l` not through a dereference of a reference | `E5006` |

- **Overlap** compares two places with the same root local projection by projection: two different fields are disjoint, so `&mut o.user` and `&mut o.email` coexist; equal projections continue; when one place is a prefix of the other, they overlap (a borrow of `o` against one of `o.user`). Two `Index` projections always overlap (the index is not known), a `Payload` is a projection like a field, and a `Deref` of a `Heap<T>` is owned memory, unlike a `Deref` of a reference.
- **A `move` of a `Copy`-typed place is a read**, as in `borrowck-moves` D3: lowering moves `Copy` temporaries such as a shared reference passed to a call, which must not conflict with the shared loans a default's generator took of it.
- **Writes are shallow, drops and moves deep.** Re-pointing `r = &b` writes the reference, not its referent, so it does not conflict with a loan of `(*r)` (and kills it, D4). A write through `*r` is an access to `(*r)`: rooted at `r`, so it never conflicts with the loan `r` itself holds, which is the point of `r`.
- **Evaluation order within a location.** A statement's or call's effects apply in evaluation order. A loan is in scope for an access when a local holding it is live after the location, or is used later in the same location. A call uses all its operands when it is made, after every operand has been evaluated, so `touch(&mut u, &mut u.name)` conflicts (both are passed together) while `x = f(r)` does not (the write to `x` comes after the call has used `r`), and `x = r + 1` for `r = &x` is accepted when `r` is not used afterwards, as the read through `r` precedes the write.
- **One report per access**, naming the earliest conflicting loan by creation order, so a conflict inside a loop is reported once.

Rejected: a single code for every conflict (the fixes differ: end the other borrow earlier, clone before moving, move the declaration out, or make the value outlive the reference), and Rust's split of borrow-borrow (E0499, E0502) and use-borrow (E0503) conflicts, whose fix is the same.

### D7. Two-phase receiver borrows come from evaluation order (follows from `mir-lowering` D10 and `borrowck-moves` D10)

A mutable receiver's borrow is taken after the arguments, so in `v.push(v.size())` the shared receiver borrow of `size` is consumed by its call before `&mut v` is taken, and nothing conflicts. No two-phase rule is added; a borrow an argument still holds when the mutable receiver is borrowed conflicts as any other (`v.pushRef(&v.count)`, `E5003`), as in Rust. By-value receivers were already moved first.

### D8. Disjoint fields, and closures capture whole bindings (follows; per-binding captures autonomous)

Disjointness is the overlap rule of D6 over field projections, which `borrowck-moves`' move paths already distinguish. Closures capture what the typed tree records, whole bindings: `{ u.name = "z" }` borrows `u` mutably, not `u.name`. Per-field captures (Rust 2021's precise captures) would need the typed tree to record capture paths and the `closures` change to lay out partial environments; they remain a possible relaxation.

### D9. Calls that return references, and the boundary with `borrowck-elision` (autonomous for the boundary)

An **input borrow** of a call is an argument whose parameter type contains a reference outside a function type (`&T`, `&mut T`, `(&T)?`), as `docs/architecture.md` §3 defines; function-typed parameters and by-value (`consuming`) receivers are not. A call's destination, when its type contains a reference, holds:

- the loans of every input borrow: the caller's half of rule 1 (one input), rule 3 (all inputs, each in its own mode, so a `&mut` source stays frozen while the result is live) and rule 4 (no input: nothing), and a sound over-approximation of rule 2 (the receiver only), since MIR does not record which operand is the receiver;
- for a call of a function value, also every loan the function value holds, since a closure may return a borrow of what it captures;
- for a default's generator, not the generator's shared borrow of an earlier argument whose type contains no reference (a by-value argument's temporary), which the call then moves into the callee: with it, `greet(s)` for `fun greet(a: String, n: &String = "anon")` would be a move while borrowed. The generator's slot (its mutable `Default` borrow) and the borrows of earlier arguments whose type contains a reference, a borrowed receiver's included, are kept, so `fun User.hi(n: &String = name)` keeps the receiver borrowed, and an aliasing default of a `mut fun` (a `&mut this` passed with a borrow of `this.name`) is a conflict.

The callee's side is `borrowck-elision`'s, which must make these assumptions true: that a function's or closure's result derives only from the inputs its rule allows (narrowing rule 2 at call sites once MIR records the receiver operand), `DanglingBorrowReturn` for borrows of locals and temporaries, that a default's generator never returns a borrow derived from a by-value earlier argument, whether a closure may return a borrow of its captures, and whether a body may store an input borrow through another one (`*slot = v` for `slot: &mut &Int, v: &Int`). Until step 9 lands, a callee can still break these assumptions, as returned borrows are unchecked today; the default-generator rule is the only assumption this change adds, and step 9 must enforce it.

Rejected: implementing rule 2 at call sites now (it needs the receiver operand in MIR, which the `ROADMAP.md` item already gives to `borrowck-elision`, together with the callee check that makes it sound); treating every call result as holding nothing until step 9, which would accept `val r = first(&x, &y); take(y); greet(r)`.

### D10. Closures capture by inferred borrow; modes are recorded in elaborated MIR (user for inferred borrows; the inference rule and its record autonomous)

A capture is **mutable** when the closure's body assigns its capture local or a field of it, writes through it, borrows it or a part of it mutably (written, argument or receiver), or builds a nested closure that captures it mutably; otherwise it is **shared**. Moves of captures cannot occur (D11). Modes are inferred innermost closure first, so a nested closure's mutable capture makes the enclosing closure's capture mutable too.

Building the closure value takes one loan per capture in that mode, at the closure's span, which the closure local holds (D4) for as long as it is live: `val f = { u.name = "z" }; greet(u.name); run(f)` is `E5003`, and `run(f); u.name = "b"` is accepted. A closure passed straight to a call (`run { greet(s) }`) is a temporary moved into the call, so its loans end with the call, since a function-typed parameter is not an input borrow (D9). A mutable capture of a `val` needs no `var`: the closure holds a unique borrow of the binding, as Rust's closures do for a `&mut` capture written through.

The modes are recorded for `closures` (step 17), which must lay out each environment: `prism_mir::CaptureMode { Shared, Mutable }`, and `Body::capture_modes: Vec<CaptureMode>`, empty as built and one per capture local in elaborated MIR (the validator checks this). `prism dump-mir --elaborated` prints each closure value's captures with their modes, `closure main::f#closure0 [&mut _1, &_2]`. Rejected: leaving the inference to `closures` (it would redo exactly this analysis), and recording the modes in `Rvalue::Closure` (the parent's rvalue, while a mode belongs to the closure body that defines it).

### D11. A closure does not move its captures: `E4017` in the type checker (autonomous; settles the "moves of variables captured by closures" follow-up)

A captured binding is borrowed by the closure (D10), so moving it inside the closure is a move out of a borrow. Whether an expression moves a captured binding depends only on its shape and on the closure nesting, so by the flow-insensitive rule of `docs/architecture.md` §3 it is a type error, like D11 of `borrowck-moves` for default values: `check_move_source` reports `CannotMoveOut` (`E4017`) for an identifier or `this` that resolves to a binding older than the innermost open closure (`builder.rs`'s `first_local`), with the source "captured variable 's', which the closure borrows" and the help of a borrowed source (`s.clone()` for a `Clone` type, otherwise taking the value as a closure parameter). `Copy` captures are copied, as before. This also covers a closure inside a default value that moves an earlier parameter. The MIR validator then checks that no operand moves a place rooted at a capture local.

So `run { take(s) }; take(s)` is `E4017` at the first `take(s)`, and the enclosing body no longer drops a moved value. Moving a captured variable in the enclosing body while the closure is live is `E5005` (D6): `val f = { greet(s) }; take(s); run(f)`. Rejected: a borrow-checker code (it would duplicate `E4017` for one more borrow source, as D11 of `borrowck-moves` argued); accepting the move as a move capture, which needs the escaping-closure design of `closures` (the closure would own `s` and could run at most once).

### D12. Capturing closures and function-typed parameters do not escape: `E5007` (user for the rule; positions and code autonomous)

A value whose type is a function type or a nullable of one **escapes** when it is written to the return place (of a function, closure or default's generator, or the initializer of a top-level `val`), used as an operand of an aggregate (a struct field or enum payload), written to a place with a projection (a field, or through a dereference) or written to a capture local. It is rejected as `EscapingClosure` (`E5007`) when it may hold a capture loan or a function input (D3), by the loan flow at that point. A closure without captures holds neither and may escape (`fun mk(): () -> Int = { 1 }`, `Button({ 1 })`); passing a capturing closure to a call is not an escape.

Writing through a dereference and writing a capture local count as escapes because the place may belong to the caller, and telling the caller's storage from the body's needs the callee-side analysis of step 9; a default value that is a capturing closure (`g: () -> Int = { a }`) is an escape because its generator returns it. Both are conservative and may be relaxed when `closures` designs escaping, move-capturing closures.

### D13. Codes, messages, labels and helps (autonomous)

The next free codes of the `E5xxx` range: `E5003` `BorrowConflict`, `E5004` `AssignToBorrowed`, `E5005` `MoveOutWhileBorrowed`, `E5006` `DroppedWhileBorrowed`, `E5007` `EscapingClosure` (`borrowck-elision` takes `E5008` on). `BorrowErrorKind` gains one variant each, with the names of the accessed and borrowed places. A place is named by its local and field names, with dereferences and payloads implicit (`(*_1).user` of `o` is `o.user`), and a temporary as "a temporary value".

| Code | Title | Primary label | Related labels |
|---|---|---|---|
| `E5003` | "cannot borrow 'x' as mutable more than once at a time"; "cannot borrow 'x' as mutable because it is also borrowed as shared"; "cannot borrow 'x' as shared because it is also borrowed as mutable"; "cannot use 'x' because it is borrowed as mutable" (when the places differ: "... because 'o.user' is borrowed as mutable") | "second mutable borrow occurs here", "mutable borrow occurs here", "shared borrow occurs here", "use of borrowed 'x'" | the loan: "first mutable borrow occurs here", "shared borrow occurs here", "mutable borrow occurs here"; the later use: "first borrow later used here", "shared borrow later used here", "mutable borrow later used here" |
| `E5004` | "cannot assign to 'x' because it is borrowed" | "'x' is assigned here while borrowed" | "'x' is borrowed here"; "borrow later used here" |
| `E5005` | "cannot move out of 's' because it is borrowed" | "'s' is moved here while borrowed" | "'s' is borrowed here"; "borrow later used here" |
| `E5006` | "'n' does not live long enough"; "temporary value dropped while borrowed" | at the loan: "borrowed value does not live long enough"; "this borrows a temporary value" | at the drop or storage end: "'n' dropped here while still borrowed"; "temporary value is freed at the end of this statement"; "borrow later used here" |
| `E5007` | "a closure that captures 's' cannot escape"; "function-typed parameter 'g' cannot escape" | by position: "returned here", "assigned to a top-level value here", "returned as a default value here", "stored in a field here", "stored in a variant here", "written through a reference here", "assigned to a captured variable here" | the capture's first use in the closure body: "'s' is captured here"; or the parameter: "'g' is a parameter here" |

- A loan made by a closure capture is labelled at the closure, "closure borrows 'u' mutably here" or "closure borrows 'u' here", with a second label at the use in the closure body that decided the mode, "borrow occurs due to this use of 'u' in the closure".
- "Later used here" is the first use, by breadth-first search from the access, of a local that holds the loan there; ", in later iteration of loop" is appended when it is reached only through a back edge (`dominators.rs`).
- Notes and helps: `E5005` helps "clone it where it moves: 's.clone()'" for a `Clone` type (`borrowck-moves`' helper); `E5006` for a temporary helps "bind the value to a local with 'val' before borrowing it, so that it lives as long as its block"; `E5007` notes "a closure borrows what it captures, so it can only be called or passed to functions while that is in scope; escaping closures that capture by move are not designed yet" and helps "pass 's' to the closure as a parameter instead of capturing it", and for a parameter notes "a function-typed parameter may be a closure that borrows the caller's variables".

The wording follows rustc's, which Prism users moving from Rust will recognise and others can read as prose.

### D14. Programs that relied on unchecked borrows (follows from the rules)

Every passing fixture and example was written before borrows were checked. During implementation, each `// expect: ok` case or example that the checker now rejects is either a scenario whose outcome this change decides, or incidental to its scenario and rewritten without changing what it exercises (a later declaration, a `.clone()`, an earlier use). Each is listed in the implementation notes of this design. Known candidates: closures in fixtures that move a capture, and defaults that are capturing closures. The `?:` fallbacks that borrow a temporary (`references/elvis-over-a-borrowed-nullable--borrowed-fallback`, `references/borrowing-a-nullable-field-without-cloning`) are not rewritten: D20 makes them sound.

### D15. Scope exits carry the span of the closing brace or of the jump (autonomous)

`E5006` labels where a borrowed local ends, which today is the span of its whole scope (a block's span starts at `{`). The parser records each block's closing brace (`ast::Block::close_brace`), the typed tree keeps it on `ExpressionKind::Block`, and MIR gives the drops and storage ends that a block runs when control reaches its end the brace's span, and those that a `return`, `break` or `continue` runs the jump's span. Scopes that are not braced blocks (a statement's temporaries, an unbraced branch) keep their span. This also gives the backend's debug information the position a drop runs at, as Rust does. Every `mir.snap` and `elaborated-mir.snap` whose block-end positions change is re-blessed, with no other change. Rejected: labelling the block's opening brace, and recomputing positions from byte offsets in the CLI.

### D16. Pipeline: exclusivity runs on the elaborated MIR, after the move check (from `borrowck-moves` D16)

`check_file` runs the move check on the built MIR of every body; if any body has an error, it returns those errors only, since conflicts on moved values are noise. Otherwise it elaborates every body, infers the capture modes of every closure body (D10), and then checks each elaborated body: liveness, loan flow, conflicts and escapes. Drops that elaboration removed are not accesses, drops guarded by a flag are (conservatively, on every path), and drop flags carry no loans. If any body has an error, it returns the sorted errors; otherwise the elaborated file with its capture modes. `prism check`, `prism dump-mir --elaborated` and the fixture suite need no change beyond the new codes.

### D17. One change, not two (autonomous)

The brief allowed splitting closures (`borrowck-closures`) from loans. They stay together because a closure is one more way of creating loans in the same flow (D3, D4): capture modes are a small inference over the closure bodies, the escape check is a pass over the same loan-flow results, and the `E4017` rule mirrors the default-value rule. Splitting would leave a release in which closures silently hold no borrows, the soundness gap the follow-up reports. The work is comparable to `borrowck-moves` (move paths, three analyses, the check, elaboration and its verifier).

### D18. Fixtures and tests (autonomous)

- Every scenario of the delta specs gets a fixture under its spec's directory (`references/`, `ownership/`, `mir/`, `diagnostics/`), one case per form; the capture-mode scenario holds an `elaborated-mir.snap` (added to `ELABORATED_SCENARIOS`).
- Unit tests in `prism-borrowck` cover place overlap and naming, liveness, loan flow (borrows, copies, reborrows, references of references, re-pointing kills, write-through, call results, generator calls, closures), capture-mode inference and the escape check, on bodies lowered from source as `check_tests.rs` does.
- `crates/prism-cli/src/diagnostics.rs` tests render one diagnostic of each new code exactly.
- `fixture_elaborated_mir_is_well_formed` and `example_elaborated_mir_is_well_formed` keep validating every passing fixture and example, now with capture modes.

### D19. Borrowed temporaries in `val` initializers (settled by `mir-lowering`; see D20)

`val r = &make()` extends the temporary to the binding (the `mir` spec), so it is accepted, and D20 extends the rule through branches. The forms still not extended, an assignment `r = &make()` and a borrowed argument whose call result outlives the statement (`val r = first(&make(), &s)`), drop their temporary at the end of the statement, which is now reported as `E5006` ("temporary value dropped while borrowed") when the reference is used afterwards, with help suggesting a `val`. Extension through assignments stays a possible relaxation in `ROADMAP.md`.

### D20. Borrowed temporaries in branch values live as long as the whole value (autonomous; found while proposing)

The MIR as lowered drops a temporary at the end of the conditional part that creates it (`mir-lowering` D7), so `greet(&u.nick ?: &"anon")`, which the `references` spec accepts ("Borrowing a nullable field without cloning", "Elvis over a borrowed nullable"), drops `"anon"` at the end of the fallback while the `?:`'s value still points at it: a dangling borrow that the checker of D6 rightly reports. Rejecting it would leave no way to write that idiom without a separate `val`. So a temporary whose written borrow is the value of a branch of `if` or `when`, of the fallback of `?:`, or of a block tail, through any nesting of these, belongs to the scope of the whole expression whose value the borrow becomes: the enclosing statement's temporary scope, or the binding's block when the expression initializes a `val` or `var` (extending `mir-lowering`'s lifetime extension through branches). Lowering finds these temporaries before lowering the expression (a walk of its value leaves through `Coerce`, branches, fallbacks and block tails, like the existing `val r = &temp` test), starts their storage before the expression, assigns each in its branch, and drops it at the end of the outer scope; since it is created on some paths only, drop elaboration guards that drop with a flag, and the move check sees only drops (not uses) on the other paths. This is Rust's lifetime extension through `if`/`match` (Rust 1.79) applied to arguments as well. Rejected: rejecting the idiom and changing the two scenarios; extending every branch temporary (it would keep unrelated values alive and need flags everywhere).

### D21. How `borrowck-elision` extends this (guidance, not decided here)

- Each input borrow argument holds an **input origin** fact at entry, like D3's function inputs; a loan of a place through a dereference of an argument derives from that argument.
- At each `Return`, the loans and origins the return place holds are checked against the rule of the signature: loans of locals or temporaries are `DanglingBorrowReturn`, an origin not allowed by rule 2 is its own code; rule 2 is narrowed at call sites once MIR records the receiver operand.
- Default generators may return only borrows of their slot, of the referents of reference-typed earlier arguments, or of top-level values (D9); closures' results follow the rule decided for function types; storing an input borrow through another input is decided there.
- New kinds take `E5008` on.

## Risks / Trade-offs

- [Over-approximating rule 2 rejects valid method calls (a method returning a borrow of its receiver keeps its other reference arguments borrowed too)] → sound and rare; `borrowck-elision` narrows it with the receiver operand.
- [Escapes through references, captures and defaults are rejected conservatively] → each has a fixture and a `ROADMAP.md` relaxation item; `closures` revisits them with move captures.
- [The loan-flow domain is locals × loans per body] → only loan-carrying locals count (D3), bodies are small, and bit sets are dense; a per-local sparse set can replace it if a body grows large.
- [Liveness that ignores drops would be unsound if a loan carrier ever owned a value with a destructor] → no loan carrier can (D5); `closures` must revisit it if move-capturing closures can borrow too.
- [Breaking: closures that move a capture, capturing closures that escape, and any program with a real borrow conflict now fail] → each was accepted only because nothing checked it; D14 lists incidental rewrites.
- [Snapshot churn from block-end spans] → mechanical re-blessing, reviewed as position-only diffs (D15).

## Migration Plan

No external users. Programs with borrow conflicts, escaping capturing closures or moves of captures now fail `prism check`. `Body` gains `capture_modes`, `ast::Block` gains `close_brace`, and typed-tree blocks keep it. Rollback is a revert.

## Open Questions

- Whether `E5003`'s read-while-mutably-borrowed case deserves its own code once tooling wants to tell it apart (kept with borrows here, D6).
- Whether escapes through a reference into the body's own locals (`*p = { x }` where `p` borrows a local `var g`) should be accepted once step 9 can tell the body's storage from the caller's.

## Implementation Notes

- D15: an unbraced block (a branch or loop body written without braces, or a block the type checker synthesizes) has no closing brace; its `close_brace` is its whole span, so its scope exits keep the span they had. A `do`/`while` body's locals, which the condition still sees, are dropped at the body's closing brace on both edges of the condition.
- D20: the walk is `Builder::extended_temporaries`; it runs where a `val` or `var` is declared and where an `if`, `when`, `?:` or block is lowered into a destination, and a temporary already given a local by an enclosing expression is not given another.
- D11: the help of a moved capture is that of any borrowed source: `s.clone()` for a `Clone` type, and taking a `&T` where the value is used. No `// expect: ok` fixture or example moved a capture, so none was rewritten.
- D10: groups 3 and 5 landed together, since elaborated MIR must record a mode per capture as soon as the validator checks it; `check_file` sets the modes after elaboration from then on. `captures::infer_capture_modes` also returns, for each capture, the use that decided its mode (the first mutable use in source order, or the capture's first use), for the labels of D13. While adding the `mir/capture-modes-in-elaborated-mir` fixture, the validator rejected the drop of the old value of a captured binding's field (`u.name = "z"` in a closure drops `_1.name`): its rule that a capture is never dropped now applies to the whole capture only, as the `mir` spec's rationale intends.
- D3: `BorrowSet::gather` reads each closure's capture modes from the elaborated file's closure bodies rather than from a separate map, and numbers the items a local may hold as the loans, then the function inputs (`borrows::Item`). A `Heap<T>` of a loan-carrying type carries loans too, for safety, though no such type passes the type checker today. `places_overlap` compares two `Index` projections as equal steps and skips the overlap test past them.
