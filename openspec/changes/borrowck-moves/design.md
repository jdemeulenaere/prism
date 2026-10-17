# Design: borrowck-moves

## Context

See `proposal.md` for motivation. This change is step 7 of "the road to the borrow checker" in `ROADMAP.md`, agreed with the user in the `/opsx:explore` session of 2026-10-17: a `prism-borrowck` crate and the `E5xxx` code range; use after move, including conditional and loop moves, and reinitialization by assignment. The planned requirements are `docs/architecture.md` §3, "Single-Owner Value Semantics and RAII" (value move on assignment, conditional and loop moves, reinitialization by assignment, consuming method moves the receiver, plain enum is move-only without the marker, String is move-only, deterministic scope exit cleanup), and §1, "Semantic Pipeline After Type Checking", scenario "Borrow checking on MIR". The rule agreed there: flow-insensitive rules are type errors (`E4xxx`), flow-sensitive ones borrow-checker errors (`E5xxx`).

`ROADMAP.md` assigns this change four follow-ups:

- from `mir-lowering` (N5): a `continue` in a `do`/`while` reaches the condition with the body's later locals live but unassigned; reading one there must be rejected;
- from `mir-lowering` (D6): scope-exit and assignment drops are inserted at lowering for every non-`Copy` value, unrefined by moves; this change "computes maybe-moved places", and someone must turn drops into static, removed or flag-guarded ones;
- from `mir-lowering` (D11): a default value that consumes an earlier parameter (`fun f(a: String, b: String = a)`) typechecks, but in its generator it is a move out of a borrow;
- from `mir-lowering` (D10) and `extension-methods`: whether a by-value receiver is read before or after the arguments; and the use-after-move half of "Consuming method moves the receiver", left in `docs/architecture.md` §3.

The orchestrator of this autonomous run decided the receiver question (D10 below). The user was not available while this change was proposed, so every other question beyond the agreed scope is decided here, marked **(autonomous)**, and logged in `docs/decisions-for-review.md`.

The consumers, each named where a decision serves it: `borrowck-exclusivity` (step 8: non-lexical borrows, disjoint fields, closure captures), `borrowck-elision` (step 9: returned borrows), and the backend (`codegen-core` to `runtime-strings-and-drops`, steps 14 to 16), which needs correct drops.

Current state:

- **MIR** (`crates/prism-mir`, the `mir` spec). `MirFile { module, types, bodies }`; each `Body` has typed `locals` (`_0` return, arguments, captures, then bindings and temporaries; each records `kind`, `mutable`, `name`, `binding`, `span`) and `blocks` of `Statement`s (`Assign(Place, Rvalue)`, `StorageLive`, `StorageDead`, `Read`) and a `Terminator` (`Goto`, `SwitchInt`, `Call { callee, arguments, parameters, destination, target }`, `Drop { place, target }`, `Return`, `Unreachable`). Places are a local plus `Deref`, `Field`, `Payload` and `Index` projections; operands `Copy`, `Move` or `Constant`; rvalues include `Borrow`, `Discriminant`, `Aggregate`, `WrapNullable`, `Subtype` and `Closure { body, captures }`. Every statement and terminator carries the `Span` of its typed-tree node. `MirFile::validate` checks the structural invariants, including storage liveness by forward dataflow. Drops are not refined by moves (D6 of `mir-lowering`); temporaries that lowering moves itself carry no drop. `call.rs` evaluates the receiver's place first, then the written arguments, then borrows, moves or copies the receiver (`PendingReceiver`).
- **CLI** (`crates/prism-cli`). `workspace.rs::validate_and_extract_abi` typechecks each file in dependency order, stops at the first file with errors (`WorkspaceError::Type`), lowers it with `prism_mir::lower_file` and discards the MIR; `dump_mir` prints the MIR of an entry. `diagnostics.rs` renders one location with a single caret per error. `fixture_tests.rs` checks codes, `typed-tree.snap` and `mir.snap` snapshots, and validates the MIR of every passing fixture and example.
- **Type checker.** `check_move_source` in `checker.rs` rejects moving a non-`Copy` value out of a field, safe navigation, index or top-level `val` (`E4017`); a parameter's default value is checked by `check_argument` before the parameter is bound.

## Goals / Non-Goals

**Goals:**
- Every use of a moved or uninitialized value is rejected with a diagnostic that says where the value moved and how to fix it.
- The analysis reads nothing but the file's MIR, so borrow checking keeps the single-file compilation and deterministic hermetic builds invariants.
- After a file passes, its MIR's drops are exact: the backend never drops a moved value and never leaks a value that is still owned.
- The crate's move paths, dataflow framework and diagnostics model are the base `borrowck-exclusivity` and `borrowck-elision` extend.
- Each of the four follow-ups is settled with a scenario.

**Non-Goals:**
- Borrow conflicts, liveness, non-lexical borrows, two-phase borrows, disjoint fields, closure capture modes and escaping closures (`borrowck-exclusivity`); returned borrows and elision (`borrowck-elision`).
- Partial moves and destructuring (move paths stay general for them), deferred `val` initialization, smart casts.
- Drop glue and running destructors: what a drop does is a layout question for `runtime-strings-and-drops`.
- Reporting errors of more than one file per `prism check` run (unchanged behavior, see Open Questions).

## Decisions

Decisions marked **(user)** were agreed in the exploration session, **(orchestrator)** was made by the orchestrator of this run, and **(autonomous)** ones were made while proposing this change and are open to review. The others follow from these or from existing rules.

### D1. A `prism-borrowck` crate checking each file's MIR alone; elaborated MIR on success (user for the crate and the range; the API and pipeline autonomous)

`crates/prism-borrowck` depends on `prism-mir`, `prism-typed-tree` (types, the type table, `Arena`) and `prism-ast` (spans). Its entry point is

```rust
pub fn check_file(file: &MirFile) -> Result<MirFile, Vec<BorrowError>>
```

which checks every body of a file's MIR as lowered (`MirPhase::Built`) and, when no body has an error, returns the file with its drops elaborated (D12, `MirPhase::Elaborated`). Errors are sorted by primary span (line, column), then code, so the output is deterministic. The check reads only the `MirFile`: the type table travels in it (D4 of `mir-lowering`), and moves need nothing from callees, since every operand already says whether it moves. So no other file's bodies, MIR or ABI is read, which keeps the single-file compilation units and deterministic hermetic builds invariants. `borrowck-elision` will read callee signatures, but those are in each `Call`'s `parameters` and destination type.

`prism check` runs it on every file it lowers, right after `lower_file` in `validate_and_extract_abi`, and, as for type errors, stops at the first file with errors, returning `WorkspaceError::Borrow { path, source, errors }`. The elaborated MIR is discarded for now, so every checked program exercises elaboration. `prism dump-mir` keeps printing the MIR as lowered, whatever its borrow errors, which is what one needs to debug the checker; `prism dump-mir --elaborated` runs the checker and prints the elaborated MIR or the errors.

Rejected: running the checker inside `prism-mir` (the MIR crate would grow every analysis of steps 7 to 9); a check that only reports and a separate elaboration entry point, which would gather move data and run the dataflow twice; continuing to the other files after a borrow error, which would make `prism check` behave differently for type and borrow errors (see Open Questions).

### D2. Use after move extends `ownership`; no new capability (autonomous)

The `ownership` spec's purpose already says that flow-sensitive rules such as use after move "belong to the borrow checker and are added here when it exists", and `references` says the same of borrow conflicts and returned-borrow lifetimes. So the language rules go there, organized by concept as the user's specs are: use after move, reinitialization and uninitialized reads to `ownership` now, conflicts and elision to `references` in steps 8 and 9. The pipeline step goes to `compilation-and-abi`, next to the typed tree's and MIR's; drop elaboration and the receiver order to `mir`, which describes what the MIR holds; the `E5xxx` range and related locations to `diagnostics`. Rejected: a `borrow-checking` capability, which would split each concept between a type-checker spec and a borrow-checker spec by the stage that enforces it.

### D3. Move paths: a local plus field projections, payloads folded into their nullable (autonomous)

A **move path** is a place that can be initialized or moved on its own: a local followed by `Field` projections. Every local of a body gets a root path (also `Copy` ones, which are never moved but can be uninitialized, D6); child paths are created on demand for the field places that a move, an initialization or a use names, so the tree is general enough for partial moves later, although no field is moved today (the `ownership` spec's whole-value rule).

```rust
define_id!(MovePathId); define_id!(MoveOutId); define_id!(InitId);
pub struct MovePath { pub place: Place, pub parent: Option<MovePathId>, pub children: Vec<MovePathId> }
pub struct Location { pub block: BlockId, pub statement: usize } // `statements.len()` is the terminator
pub struct MoveOut { pub path: MovePathId, pub location: Location, pub span: Span }
pub enum InitKind { Entry, Assign, CallDestination }
pub struct Init { pub path: MovePathId, pub location: Location, pub kind: InitKind, pub span: Span }
pub struct StorageStart { pub local: LocalId, pub location: Location, pub span: Span }
pub struct MoveData { /* paths, moves, inits, storage starts, and per-location indexes */ }
```

How places map to paths:

- **`Payload` is folded into its nullable**: a move of `(u as payload)` is a move of `u`. A `T?` owns nothing but its payload, so after the payload moves the nullable owns nothing, on the non-null path, and a `null` one owned nothing; treating it as moved needs no partial-move state and makes drop elaboration exact. This is what `u?.toAccount()` and `?:` do (the payload of a nullable place or temporary is moved out). A later `u == null` is then a use of a possibly moved value, as in Rust, until smart casts exist.
- **`Deref` ends the path**: a place through a dereference is not owned by the body. A move through a dereference is impossible (the type checker rejects moving out of a borrow or `Heap<T>`, `E4017`, and D11 removes the last case); the MIR validator checks it, and `MoveData` treats a violation as an internal error. Uses through a dereference are uses of the reference's path (D5).
- **`Index` ends the path too**: an index reads a `Char`, which is `Copy`; the index local is a use.
- **`Copy` values never move**: a `Move` operand of a place whose type is `Copy` (lowering emits these only for temporaries) is treated as a copy.

Rejected: tracking whole locals only (step 8's disjoint fields and any future partial move would rebuild the structure); a separate `Payload` child path, which would need drop elaboration to open-drop nullables for a state that is never observable.

### D4. A bit-set dataflow framework and two forward analyses (autonomous)

`dataflow.rs` provides a `BitSet` and a generic fixpoint solver over a body's CFG, with a direction (forward now; backward for `borrowck-exclusivity`'s liveness), a join (union), the entry state, and per-statement and per-terminator transfer functions, iterating blocks in reverse postorder until no block-entry state changes. A `ResultsCursor` replays a block's effects to give the state before any location. Effects within one statement apply in evaluation order (D5), so `Pair(s, s)` sees its second operand after the first moved.

Two analyses, both over move paths or events of one body:

- **`MaybeInitialized`** (bits: move paths): at entry the arguments and captures and their children are set; an initialization sets the path and its children; a move, `StorageLive` or `StorageDead` of a local clears it and its children. A `Call`'s destination is initialized by the call terminator, which only has its normal edge (MIR has no unwinding). It answers "possibly" in diagnostics and "initialized on some path" for drop elaboration.
- **`ReachingUninitializations`** (bits: events, each a `MoveOut` or a `StorageStart`): a move or `StorageLive` adds its event; an initialization of a path removes the events of that path and its children; `StorageLive` and `StorageDead` of a local remove its earlier events. A path is possibly uninitialized at a location exactly when an event of the path, an ancestor or a child reaches it, and the reaching events are what the diagnostics label (Rust's "ever-initialized" plus move-out sets, merged).

A path is definitely initialized where `MaybeInitialized` has it and `ReachingUninitializations` has none of its events. No third analysis is needed: locals other than arguments are only used and dropped after their `StorageLive` (the validator's storage invariant), which is itself an event.

Rejected: a hand-written worklist per analysis (steps 8 and 9 add three more); SSA or a lattice library dependency, for a few hundred lines of bit sets.

### D5. What uses, initializes and moves a place (autonomous)

Visiting each statement and terminator in evaluation order:

| MIR | Effect |
|---|---|
| `Operand::Copy(p)`, `Rvalue::Discriminant(p)`, `Read(p)`, `Callee::Value(p)`, a `SwitchInt` operand | use of `p` |
| `Operand::Move(p)` | use of `p`, then a move of its path (unless `Copy`, D3) |
| `Rvalue::Borrow { place }` | use of `place` ("borrowed") |
| `Rvalue::Closure { captures }` | use of each captured place ("captured", D13) |
| any place with a `Deref` or `Index` projection | use of the reference's (or index local's) path |
| `Assign(local, _)` (no projection) | initialization of the local, after the rvalue's effects |
| `Assign(p.field, _)` | use of `p` ("assigned"), since the field's struct must hold a value |
| `Call { destination: local }` | initialization of the local, after the operands' effects |
| `StorageLive(l)`, `StorageDead(l)` | `l` becomes uninitialized (an event for `StorageLive`) |
| `Drop { place }` | none: drops are not uses (D12) |

Arguments and captures are initialized at entry, everything else uninitialized; the return place is never read. A use of a path checks the path, its ancestors and its children (a moved child would make a whole-value use partial).

Drops are not uses because a drop of a moved value is exactly what elaboration removes; `borrowck-exclusivity` will treat the drops that remain after elaboration as uses (D16).

### D6. The use check: `E5001` and `E5002` (user for the range; codes and classification autonomous)

At each use (in reverse postorder of blocks, statements in order) the cursor of `ReachingUninitializations` gives the events of the used path, its ancestors and children that reach it:

- If any is a move, the use is **`E5001` `UseAfterMove`**, "use of moved value 's'", or "use of possibly moved value 's'" when `MaybeInitialized` also has the path (moved on some paths only). Related locations: each reaching move (D7).
- Otherwise, if any is a `StorageStart`, it is **`E5002` `UseOfUninitialized`**, "use of uninitialized value 'x'" or "use of possibly uninitialized value 'x'". Related locations: each reaching `StorageStart`.

The value is named by its local's `name` and the path's field names (`u.name`, for later partial moves); `this` is `this`. An error on an unnamed temporary is an internal error, since lowering uses each temporary once. A use is reported once per used path and set of reaching events, the first in visiting order: as in Rust, a use reached by the same moves as an already reported use of the same value is not reported again. Fixtures check the set of codes, and CLI tests the positions. The two codes are separate kinds because the fixes differ: clone, copy or reassign a moved value; move a declaration before a jump for an uninitialized one (Rust's `E0382` and `E0381`).

`E5001` is the first code of the new range, and the borrow checker's later codes (conflicting borrow, move while borrowed, escaping closure, `DanglingBorrowReturn`, ...) take the next ones.

### D7. Labels: "in previous iteration of loop" from back edges; skipped declarations (autonomous)

The primary label is the use's kind: "value used here after move" (reads, copies, moves, calls), "value borrowed here after move", "value captured here after move" and "value assigned here after move" (a field write); for `E5002`, "'x' used here before it is initialized" or "'x' used here, but it is not initialized on every path".

A reaching move is labelled "value moved here", or "value moved here, in previous iteration of loop" when it does not reach the use along a path without back edges. A back edge is an edge to a block that dominates its source; `dominators.rs` computes dominators by the Cooper-Harvey-Kennedy iteration over reverse postorder (MIR from structured source is reducible), which `borrowck-exclusivity` reuses. Within one block, a move reaches a use without a back edge when it comes earlier (an earlier statement, or an earlier operand of the same statement); so in `while (c) take(s)`, the move is the use's own operand, reached only around the loop, and the related location is the same `take(s)` labelled as a previous iteration, as Rust does.

A reaching `StorageStart` is labelled "the declaration of 'x' is skipped here". Today the only `StorageLive` that reaches a use before an assignment is the one that lowering puts on a `do`/`while` `continue` path (N5 of `mir-lowering`, with the `continue`'s span), because every other `StorageLive` is followed at once by the local's assignment. When deferred `val` initialization arrives, a declaration without a value gets its own label then.

Related locations are listed in source order.

### D8. Notes and help lines (autonomous; the two suggestions from `docs/architecture.md` §3, user)

For `E5001`:

- a note: "'s' has type 'String', which is not 'Copy'";
- `: Copy` help, when the type is a struct or enum without `: Copy` whose definition is in the type table and whose every field or variant payload is `Copy`: "add ': Copy' to the declaration of 'Color' to copy its values instead of moving them" (naming the module, "in 'geo/types'", for an imported type). This is the "Plain enum is move-only without the marker" suggestion; a type that could not be `Copy` gets no such help;
- `.clone()` help, when the type is `Clone`: "clone it where it moves: 's.clone()'" (`s?.clone()` for a nullable), the "String is move-only" suggestion;
- for a move in a previous iteration of a `var`: "assign 's' a new value before the next iteration".

For `E5002`: "declare 'x' before the jump that skips it".

`E4017`'s help style is kept (borrow, then clone). Suggesting a borrowed parameter type at a by-value call (Rust's "consider borrowing here") was rejected for now: a parameter is often by-value on purpose, and the callee may be in another file.

### D9. A structured diagnostic, rendered with related locations (autonomous)

```rust
pub struct BorrowError { pub kind: BorrowErrorKind, pub primary: Label, pub related: Vec<Label>, pub notes: Vec<String>, pub helps: Vec<String> }
pub struct Label { pub span: Span, pub message: String }
pub enum BorrowErrorKind { UseAfterMove { name: String, possibly: bool }, UseOfUninitialized { name: String, possibly: bool } }
impl BorrowError { pub fn code(&self) -> &'static str; pub fn message(&self) -> String; }
```

`prism-cli/src/diagnostics.rs` gains `format_borrow_errors`, which prints the header, the primary location and a snippet whose caret line underlines the span's first line (at least one `^`) followed by the label, then each related location with its own `-->` line, snippet and label, then `= note:` and `= help:` lines:

```text
error[E5001]: use of moved value 's'
  --> main.prism:2:34
    2 | fun f(s: String) { take(s); take(s) }
      |                                  ^ value used here after move
  --> main.prism:2:25
    2 | fun f(s: String) { take(s); take(s) }
      |                         ^ value moved here
   = note: 's' has type 'String', which is not 'Copy'
   = help: clone it where it moves: 's.clone()'
```

Spans come straight from MIR: every statement and terminator carries the span of the typed-tree node it was lowered from, and argument and receiver temporaries are assigned at the argument's own span, so labels point at the operand, not at the whole call. A `Label` per location keeps the model open for steps 8 and 9 (a borrow's creation, its later use) and for other stages that may adopt it. Rejected: reusing `TypeError`'s flat shape (one span and an optional help), which cannot say where a value moved.

### D10. By-value receivers are evaluated before the arguments (orchestrator; replaces `mir-lowering` D10 for them)

The receiver of a call that moves or copies it, a `consuming fun` receiver or a `Copy` receiver copied out of a reference, is moved or copied first, before the written arguments: strict left-to-right evaluation, as in Kotlin and Rust (where only auto-referenced `&mut self` receivers get two-phase borrows). A borrowed receiver keeps `mir-lowering`'s two-phase shape: its place is evaluated first and the borrow taken after the arguments, which is what makes `v.push(v.size())` acceptable by liveness alone in step 8. So `s.consume(f(&mut s))` is now `E5001` at `&mut s`; before, it moved the `s` that `f` had mutated, which no reader of the source expects.

In MIR, `call.rs`'s `receiver_place` returns a pending receiver only for a `Receiver`-origin borrow; a `Move` or `Copy` receiver is lowered by `argument` in `evaluation_order`, where the typed tree lists the receiver first, so it is moved or copied into a temporary at the receiver's span before any argument. The `PendingReceiver::Move` and `PendingReceiver::Copy` variants go away. The `mir` spec's "Call Arguments Are Evaluated in Order" and `docs/architecture.md` §4's "Method Call Evaluation Semantics" say so, and the `mir.snap` of fixtures with consuming receivers are re-blessed.

### D11. A default value may not move an earlier parameter: a type error (autonomous; settles the `mir-lowering` D11 follow-up)

A default's generator receives shared borrows of `this` and the earlier arguments (D11 of `mir-lowering`), so `fun f(a: String, b: String = a)` would move out of a borrow. Whether it does depends only on the expression's shape and type, so by `docs/architecture.md` §3's rule it is a type error: `check_move_source` rejects a consumed `a` or `this` that resolves to a parameter of the function whose default is being checked (outside closures in the default, whose captures `borrowck-exclusivity` decides), as `CannotMoveOut` (`E4017`) with the source "parameter 'a', which a default value borrows" and the help of a borrow source (`a.clone()` for a `Clone` type, or taking a `&String` instead). `b: &String = a` and `b: String = a.clone()` stay valid. The MIR validator then checks that no operand moves a place through a dereference, which the borrow checker relies on (D3). Rejected: a borrow-checker code for moves out of dereferences, which would duplicate `E4017` for the one case the type checker missed, and accepting it with a copy, which `String` cannot have.

### D12. Drop elaboration belongs here, with drop flags as locals (autonomous; moves "drop elaboration on MIR" from `runtime-strings-and-drops`)

Elaboration needs exactly this change's move paths and `MaybeInitialized`/`ReachingUninitializations` results (Rust's `ElaborateDrops` reuses the borrow checker's `MoveData` the same way), so it runs at the end of `check_file`, per body, on the MIR that just passed:

- A `Drop` of a place through a `Deref` (the old referent of `*r = v`) or of a field of a local (the old value of `u.name = v`, whose struct the use check proved initialized) stays.
- A `Drop` of a whole local stays where the local is definitely initialized; becomes a `Goto` to its target where it is not possibly initialized (moved on every path); otherwise it is **guarded by the local's drop flag**: the block ends in `SwitchInt(copy flag) [0 -> target, otherwise -> d]`, and a new block `d` holds the `Drop`.
- A local with a guarded drop gets one drop flag, a new local of `LocalKind::DropFlag` and type `Bool`, without storage markers. It is assigned `const true` or `const false` at the start of `bb0` (whether it is initialized on entry: true for arguments), `const true` after each initialization (for a call destination, at the start of the call's target, splitting the edge into a new block if the target has other predecessors), and `const false` before each move of the local and after each `StorageLive` and `StorageDead` of it. So the flag is true exactly when the local holds a value, the property the analyses compute statically.
- A partially moved local cannot occur (no field moves, payloads folded, D3); elaboration asserts it, and the partial-moves change adds open drops.
- Afterwards the goto chains are merged with `mir-lowering`'s block merging, made public in `prism-mir`, and the file's `phase` becomes `MirPhase::Elaborated`.

`MirFile` gains `phase: MirPhase { Built, Elaborated }`; `Body::is_unmarked` becomes kind-based (return place, arguments, captures and drop flags); the validator checks that drop flags appear only in elaborated MIR, are `Bool`, are only assigned constants and only read by `SwitchInt`; and the dump prints `phase elaborated` after the `module` line and `(drop flag)` for flag locals.

Why now: the analyses are fresh and identical; `borrowck-exclusivity` should check the drops that will run, which the elaborated MIR has (D16); the backend then consumes MIR that the checker has fully seen, rather than a pass that would sit untested until step 16; and the "Deterministic scope exit cleanup" scenario becomes testable on MIR snapshots. `runtime-strings-and-drops` keeps drop glue (what dropping a `String`, a struct or a `Heap<T>` does) and the runtime half of that scenario, so `ROADMAP.md` step 16 drops "drop elaboration on MIR" on archive. Rejected: deferring elaboration to step 16, which would gather move data a second time or couple a backend crate to the borrow checker's internals; elaborating inside `prism-mir`, which would have to host the move data and dataflow; drop flags as ordinary temporaries with storage markers, which would need markers around the whole body and confuse the storage invariant; Rust-style open drops of nullables, unnecessary once payload moves fold into the nullable.

### D13. Closures: creating one uses its captures; moves inside a closure are local to its body (autonomous)

Building a closure value (`Rvalue::Closure { captures }`) uses each captured place, so `take(s); run { greet(s) }` is `E5001` ("value captured here after move"). A closure's body is checked on its own: its capture locals are initialized on entry like arguments, and a move of one is tracked within that body. Whether a closure may move a capture at all (it holds an inferred borrow), and a move of a captured variable while the closure is live, are `borrowck-exclusivity`'s (the `ROADMAP.md` item "Moves of variables captured by closures"). Capture locals are never dropped by the closure (D13 of `mir-lowering`), so elaboration never sees them.

### D14. Programs that relied on use after move (follows from the rules)

Every passing fixture and example was written before uses after move were checked. During implementation, each `// expect: ok` case or example that the checker now rejects is either a scenario whose outcome this change decides (known: `mir/continue-in-a-do-while-reaches-the-condition` becomes `// expect: E5002`, keeping its `mir.snap`) or incidental to its scenario, and is then rewritten (a `.clone()`, a fresh binding) without changing what it exercises. Each rewrite is listed in the implementation notes of this design.

### D15. Fixtures, snapshots and self-checks (autonomous)

- Every scenario of the delta specs gets a fixture under its spec's directory (`ownership/`, `mir/`, `compilation-and-abi/`, `diagnostics/`), one case per form.
- A case may hold `elaborated-mir.snap`: the expected output of `prism dump-mir --elaborated` on its entry (`prism_cli::dump_elaborated_mir`), compared by `fixture_tests.rs` like `mir.snap` and re-blessed by `PRISM_BLESS=1`. The cases of "Drops Are Elaborated After Borrow Checking" must hold one.
- `fixture_elaborated_mir_is_well_formed` and `example_elaborated_mir_is_well_formed` elaborate and `validate()` every passing fixture and example, and also run `prism_borrowck::verify_elaborated`, which re-gathers move data on the elaborated body and checks that every unguarded drop of a local finds it definitely initialized and every guarded drop's flag mirrors `MaybeInitialized`. This is the only check of elaboration before the backend executes it.
- Unit tests in `prism-borrowck` cover the dataflow framework, dominators and back edges, move-path folding and each analysis on small bodies lowered from source.
- `cli_workspace_tests.rs` checks the rendered `E5001` (related location, note, help) and `E5002` (the skipped declaration) and `dump-mir --elaborated`.

### D16. How `borrowck-exclusivity` and `borrowck-elision` extend the crate (guidance, not decided here)

- **Order in `check_file`**: moves and initialization (this change) on the built MIR, then elaboration, then the exclusivity check on the elaborated MIR, so drops that remain are real uses of what they drop (a dropped value must not be borrowed), and a drop elaboration removed is no use. Move errors come first, since conflicts on a moved value are noise.
- **Exclusivity** adds a borrow set (each `Rvalue::Borrow` as a loan: place, mutability, location, origin), liveness of reference-typed locals as a backward analysis of the same framework, regions as the sets of locations where a loan's reference is live (non-lexical borrows), a forward "borrows in scope" analysis, and the access check: an access to a place conflicts with an in-scope loan of an overlapping place, where overlap compares projections (so `o.user` and `o.email` are disjoint, from D3's field paths). A move while borrowed is an access by a `MoveOut`. No two-phase rule is needed (D10 and `mir-lowering` D10). Capture modes come from the uses of each capture local in the closure's body (D13).
- **Elision** checks each body's returned borrow against its input borrows, using a `Call`'s `parameters` and destination type at call sites. Rule 2 needs to know which operand is a receiver, which MIR's `Call` does not record yet; that change adds it.
- New error kinds join `BorrowErrorKind` with the next `E5xxx` codes, using `Label`s for the loan and the conflicting access.

### Implementation notes

- **D11: a default value is now a consuming position.** The type checker checked a default value with `check_argument(.., Origin::Value)`, which did not mark it consumed, so `fun f(a: String, b: String = a)` produced a typed tree whose default read `a` in place, and lowering it panicked ("a String place read in place is used as a value"). The default is now checked in a consuming position, as the argument it stands for, which makes `check_move_source` see the move and report `E4017`. For a shared receiver, `fun User.g(u: User = this)` was already `E4017` ("cannot move out of borrow 'this'", since `this` is `&User` there); the new source wording applies to an earlier by-value parameter and to a `consuming` receiver.
- **D14: no incidental rewrite was needed.** With the checker wired into `prism check`, the only `// expect: ok` fixture it rejects is `mir/continue-in-a-do-while-reaches-the-condition`, which now expects `E5002` and keeps its `mir.snap`. No other fixture and no file under `examples/` used a value after moving it.
- **D9: labels at a statement's span.** An operand that a statement moves or borrows in place, without a temporary of its own, is labelled at the statement's span: both moves of `Pair(s, s)` point at the whole construction, and a use by `s == "a"` at the whole comparison. Arguments and receivers have their own temporaries and spans, as D9 expects. Refining these spans is a lowering change that needs no change to the checker.
- **D15: how `verify_elaborated` checks flags.** Rather than comparing each flag with `MaybeInitialized`, which is path-insensitive, it solves a forward analysis over the pairs (value of the local's flag, whether the local holds a value) that may hold at each point, on the elaborated MIR alone. Every drop of a whole local must find no pair without a value (for a guarded drop, among the pairs whose flag is set), and at every guarding switch each pair must have its flag equal to its initialization. This is stronger than D15's wording and needs no knowledge of how elaboration placed the flags. Found in review: a drop must also take the value, or a value used or dropped again after its drop goes unnoticed (the `s = s` bug below), and nothing checked that values are not leaked. So the analysis tracks what each local holds (nothing, a value, or `null`, which a nullable holds on the edge where its tag test finds it so and which owns nothing), a drop leaves nothing, a switch on a drop flag passes each edge only the states whose flag matches it (an edge hook of the dataflow framework), and the check also rejects a use of a local that may hold nothing, and an assignment, a `StorageDead` or (for an argument) a return where a value that needs a drop may still be held.
- **Review: `s = s` dropped the value it moved.** An assignment's value was a move read in place after the drop of the old value (`drop _1; _1 = move _1`), and elaboration kept that drop, since `s` holds a value there. `lower_assign` now moves a named place into a temporary before the drop, as the `mir` spec's "Assignments Drop the Old Value" already required; the scenario "Assigning a variable to itself" pins it down.
- **D8: helps for a nullable.** The `: Copy` help looks through one nullable (`p: Point?` suggests `: Copy` on `Point`), and the `.clone()` help reads `s?.clone()` for a nullable, as D8 says.

## Risks / Trade-offs

- [Labels are only as precise as MIR spans; a statement lowered at a whole call's span would label the call] → argument and receiver temporaries carry their operand's span; CLI tests check two rendered diagnostics; spans can be refined in lowering without touching the checker.
- [Folding payload moves into the nullable rejects `u?.toAccount(); if (u == null) ...` even where `u` is `null`] → this is as conservative as Rust; smart casts or flow typing can relax it later without breaking programs.
- [Breaking: consuming receivers now move before the arguments, and a default may no longer move an earlier parameter] → both affected forms were accepted only because nothing checked them; each has a fixture, and the old behavior was a use after move or a move out of a borrow.
- [Elaboration bugs stay invisible until the backend runs MIR] → `verify_elaborated` re-checks every passing fixture and example, the validator checks drop flags, and elaborated snapshots cover each scenario.
- [The `continue` case is the only uninitialized read, so `E5002` is a narrow code] → deferred `val` initialization will reuse it, and keeping uninitialized reads apart from moves gives each its own fix.
- [Fixture churn: passing fixtures that used moved values must be rewritten] → D14 keeps each scenario and lists every rewrite.
- [De-duplication hides repeated uses of a value moved by the same moves] → one error per value and set of moves is enough to fix them all, and fixtures compare code sets.

## Migration Plan

No external users. Programs that used a value after moving it, consumed a receiver mutated by its own arguments, or moved an earlier parameter in a default now fail `prism check`. `MirFile` gains `phase` and `LocalKind` gains `DropFlag`; callers of `lower_file` see `MirPhase::Built`. Rollback is a revert.

## Open Questions

- Whether `prism check` should report the errors of every file instead of stopping at the first file with errors, for type and borrow errors alike (useful for the LSP).
- Whether a move label should name its reason ("moved by the `consuming` receiver of `toAccount`"), which needs MIR to record why an operand moves.
- Whether move paths and the dataflow framework should become their own crate once the backend needs them.
