# Design: borrowck-elision

## Context

See `proposal.md` for motivation. This change is step 9, the last, of "the road to the borrow checker" in `ROADMAP.md`, agreed with the user in the `/opsx:explore` session of 2026-10-17. The planned requirement is `docs/architecture.md` §3, "The Four Lifetime Elision Rules": a returned borrow's lifetime is inferred from the signature alone, so callers in other files apply the rules to the imported ABI, and function types elide by the same rules; input borrows are parameters whose type contains a reference (`&T`, `&mut T`, `(&T)?`) and the receiver of a non-`consuming` method, while function-typed parameters and `consuming` receivers are not; a borrow of a top-level `val` is valid everywhere, including from a function with no input borrows; rule 1 (one input borrow) ties the result to it; rule 2 (a borrowed receiver, whatever the other parameters) ties it to the receiver only, and returning a borrow derived from another parameter is rejected with its own `E5xxx` code; rule 3 (several input borrows) ties it to every one, each in its own mode, a `&mut` source frozen while the result is live; rule 4 (none) and returning a borrow of a local or temporary under any rule are `DanglingBorrowReturn`. The design note says elision needs nothing new in the ABI.

`ROADMAP.md` assigns this change these follow-ups:

- from `borrowck-moves` D16: record which operand of a MIR `Call` is the receiver, which rule 2 needs (D3);
- from `borrowck-exclusivity` D9: make true on the callee's side what call sites assume, namely that a function's or closure's returned borrow derives only from the inputs its rule allows (narrowing rule 2 at call sites), that a default's generator never returns a borrow of a by-value earlier argument, and that a returned borrow never derives from a function-typed argument's result; and decide whether a closure may return borrows of its captures and whether a body may store an input borrow through another input (`*slot = v` for `slot: &mut &Int, v: &Int`) (D2, D8 to D11);
- from the review of `borrowck-exclusivity`: `u.rename()` for `mut fun User.rename(n: &String = name)` is `E5005` "cannot move out of a temporary value" at the receiver (D10), and an `E5006` for a borrow written through a `&mut` argument labels its later use at the function's signature (D12);
- lending a `(&mut T)?` where a `(&mut T)?` is expected crashes MIR lowering (D13, an orchestrator decision).

The user was not available while this change was proposed. The orchestrator of this autonomous run decided D13; every other question beyond the agreed scope is decided here, marked **(autonomous)**, and logged in `docs/decisions-for-review.md`.

Current state:

- **Caller side** (`crates/prism-borrowck/src/loan_flow.rs`, D9 of `borrowck-exclusivity`). `LoanFlow::call_items` gives a call's destination, when its type contains a reference outside a function type (`places::is_input_borrow`), the items of every argument whose parameter is an input borrow, of a `Callee::Value` callee, and, for a `Callee::Default` call, not those of a shared argument whose referent carries no loans (a by-value earlier argument's borrow). So a method result keeps every reference argument borrowed (`val c = t.captain(&s); s = "y"; greet(c)` is `E5004`), and `val h = u.hi()` for `fun User.hi(n: &String = name): &String` is `E5006`, because the result keeps the generator's borrow of the receiver's temporary, which ends with the statement.
- **Callee side.** Nothing: `Liveness` does not use the return place at `Return` (D5 of `borrowck-exclusivity`), so `fun f(): &Int { val n = 1; return &n }`, `val R: &Int = &1`, `fun Team.other(o: &String): &String = o`, `fun call(h: () -> &Int): &Int = h()`, a closure returning `&m` of its local `m`, and `fun m(a: String, b: &String = a)` all pass `prism check`.
- **Inputs.** `BorrowSet::gather` gives an `Item::Input(local)` at entry to each argument or capture whose type contains a function type (a function input) and each argument through which loans can be written (`places::writes_loans_through`, a referent input). Reference arguments hold nothing at entry.
- **Arguments at the end.** `returned_locals` makes capture locals and `&mut R` arguments live at every `Return`, so a body storing a borrow of its own local there is `E5006` when the local ends. An argument has no storage markers, and one whose type needs no drop is never dropped, so `fun keep(slot: &mut &Int, v: Int) { *slot = &v }` and a closure writing `r = &x` for its `Int` parameter `x` pass, leaving the caller a dangling borrow. With a `String` parameter the `E5006` labels the drop and the later use at the function's whole span, the spans of the drop and `Return` terminators.
- **Receivers in MIR.** `TerminatorKind::Call` does not say whether its first operand is a receiver, but its `Callee::Function(path)` does: `DeclarationPath::receiver` is `Some` exactly for an extension (the typed-tree validator checks that a call has a receiver exactly when its function has one), and an extension cannot be named as a value (`E4006`). Likewise `BodyOwner::Function(path)` of an extension's body.
- **Default generators** (`crates/prism-mir/src/build/call.rs`, the `mir` spec). A generator's arguments are a shared borrow `&P` of each earlier argument's temporary, the receiver's included (`P` is `&User` for a `fun` receiver, `&mut User` for a `mut fun`, `User` for a `consuming fun`), then, for a reference-typed parameter, a `&mut R?` slot, the only mutable-reference argument. A call passes the same types as `parameters`.
- **Top-level values.** A borrow of a top-level `val` is `Constant::TopLevelValue`, which holds no loans, so `&DEFAULT` (lowered as `_1 = const &DEFAULT; _0 = &(*_1)`) carries nothing.
- **`(&mut T)?` arguments.** The typed tree reads such a place in place where a `(&mut T)?` is expected (`edit(m)`), which the typed-tree spec allows ("a place used anywhere else is read in place"), but `Builder::operand` (`crates/prism-mir/src/build/place.rs`) only reads `Copy` places in place and panics. Lent as a `(&T)?`, the place is a `Subtype` coercion that `lower_reborrow` (`build/comparison.rs`) lowers through `optional_borrow`.

## Goals / Non-Goals

**Goals:**
- Caller and callee agree exactly, by construction, on what a returned borrow may come from, for functions, methods, function values, closures and default generators, from parameter types and the receiver alone, so the single-file compilation units and deterministic hermetic builds invariants hold with no new ABI data.
- Every way a callee can hand its caller a borrow (the return place, a `&mut R` parameter, a captured variable) is checked against what the caller assumes.
- Each rejection names the borrowed value or the parameter and the rule, so the fix is evident.

**Non-Goals:**
- Lifetime annotations or any way to tie a result to a chosen subset of inputs (the zero-cost memory-safety invariant has no lifetime annotations).
- Counting function-typed parameters as input borrows, so that `fun apply(g: (&Int) -> &Int, v: &Int): &Int = g(v)` could be accepted (a possible relaxation, D9).
- Escaping, move-capturing closures, and the escape relaxations left to `closures`; lifetime extension through assignments; promoting constant temporaries (`val R: &Int = &1`) to static storage.

## Decisions

Decisions marked **(user)** were agreed in the exploration session, **(orchestrator)** was made by the orchestrator of this run, and **(autonomous)** ones were made while proposing this change and are open to review. The others follow from these or from existing rules.

### D1. The callee's rules extend `references` (follows from its Purpose)

The `references` spec's Purpose says "what a function may return a borrow of (the callee's side of the elision rules) is added here by `borrowck-elision`". The rules for returned borrows, function types, defaults and stored borrows join it, next to "Calls Returning References Keep Their Inputs Borrowed", which this change narrows; the `(&mut T)?` reborrow extends "Passing a Mutable Borrow Reborrows It" there and "Optional Mutable Borrows Are Reborrowed" in `mir`; the new codes go to the `diagnostics` catalog. On archive the elision requirement leaves `docs/architecture.md`.

### D2. One function decides what a result is tied to, for callers and callees (user for the rules; the single function autonomous)

A new module `crates/prism-borrowck/src/elision.rs` holds the rules as one function of a signature:

```rust
pub enum Signature { Function { has_receiver: bool }, FunctionValue, Default }
pub enum Rule { NoInput, Single, Receiver, Multiple, FunctionValue, Default }
/// The rule, and the positions of the parameters a returned borrow is tied to.
pub fn tied_inputs(signature: Signature, parameters: &[Type]) -> (Rule, Vec<usize>)
```

- `Function { has_receiver: true }` with a receiver that is an input borrow (`&T` or `&mut T`): rule 2, the receiver (position 0) only.
- Any other `Function`: every position whose type is an input borrow (`places::is_input_borrow`): rule 4 for none, rule 1 for one, rule 3 for several. A `consuming` receiver (`T`) is not an input borrow, so such a method follows rules 1, 3 and 4 over its other parameters.
- `FunctionValue`: rules 1, 3 or 4 over the parameters, plus the function value itself (D8).
- `Default`: the slot (the `&mut R?` argument) and each earlier argument `&P` whose `P` is a shared input borrow (`&T`, `(&T)?`, a `fun` receiver `&User`), D10.

The caller uses it in `LoanFlow::call_items`, from the call's `parameters` and its callee (`Callee::Function(path)` gives `has_receiver` by D3, `Callee::Builtin` is a method with a shared receiver, `Callee::Value` is a function value, `Callee::Default` a default); the callee uses it in the return check (D5), from its arguments' types and its owner. Since both read the same types (an imported callee's parameter types come from the ABI, which also names the receiver), they agree exactly:

| Callee | The caller's result keeps borrowed | The callee may return borrows derived from |
|---|---|---|
| method with a borrowed receiver (`fun`, `mut fun`) | what the receiver borrows | the receiver (rule 2) |
| other function, `consuming fun` included | what every input borrow borrows | every input borrow (rules 1 and 3), none (rule 4) |
| function value | what every input-borrow argument borrows, and the function value's borrows | a closure: its reference parameters and its captures; a function: as above, never rule 2 (D8) |
| default value | its slot, and what each shared reference earlier argument borrows | its slot, and through each shared reference earlier argument (D10) |
| any | — | top-level values, which hold nothing (D7) |

Nothing may be returned that borrows a local, a temporary or a by-value parameter (`E5008`), or that derives from an input the table does not list (`E5009`), including a function-typed parameter (D9). Rejected: computing the caller's and the callee's sets in two places (`borrowck-exclusivity` did this and needed a "boundary" decision; one function cannot drift).

### D3. Rule 2's receiver operand comes from the callee's path (autonomous; settles the `borrowck-moves` D16 item)

MIR already records it: a call's first operand is a receiver exactly when its callee is `Callee::Function(path)` with `path.receiver` set (or the built-in `clone`), and a body's first argument exactly when its owner is `BodyOwner::Function(path)` with `path.receiver` set. `prism-mir` gains `Callee::has_receiver()` and `BodyOwner::has_receiver()` to say so in one place, and the `mir` spec's "Calls Name Their Callee" states it. Rejected: a `has_receiver` field on `TerminatorKind::Call`, which would duplicate what the path says and need a validator rule to keep the two equal; the receiver's parameter type alone (a `&T` first parameter is not necessarily a receiver).

### D4. Every argument that carries loans holds its own input (autonomous)

`borrowck-exclusivity`'s D21 suggested an "input origin" per input borrow. `BorrowSet::gather` gives an `Item::Input(local)` at entry to every argument whose type carries loans (`places::carries_loans`: references, function types and nullables of them), and, as today, to every capture whose type contains a function type. The input stands for whatever the caller passed, so one item serves the three roles it had and gains one: a function input (escapes, D12 of `borrowck-exclusivity`), a referent input (writes through `&mut R`), and now the origin of a returned borrow. Each role looks at the argument's type, as `places::contains_function` and `places::writes_loans_through` already do, so the escape check, the conflict check (inputs never conflict) and `referents` are unchanged. A borrow of a place through a dereference of an argument (`&(*_1).lead`) holds that argument's input too, since a borrow holds the root local's items (D4 of `borrowck-exclusivity`). Rejected: a separate item kind per role (three facts for the same caller value); origins for capture locals of reference type, which need none (D8).

### D5. The callee's check runs at each write of the return place (autonomous)

A new module `crates/prism-borrowck/src/returns.rs`, `check_returns(file, body, borrows, flow)`, visits each statement assigning the whole return place and each call whose destination is the return place, in a body whose return type is an input borrow (a function-typed result is the escape check's, D12 of `borrowck-exclusivity`). With the loan flow before that location, it takes the items the written value carries (`rvalue_items`, or the call's items for a call) and classifies each:

- **A loan of a place reached through a dereference of a reference from its root local** (`places::extends_through_reference`): not judged itself, since the written value also holds the root local's items, which are judged. In a default generator, a loan of `(*_i)` or a field of it, for an earlier argument `_i` (the caller's argument temporary), not through a further reference, borrows the argument itself (D10).
- **Any other loan** borrows memory the body owns: a local, a temporary, a by-value parameter, or, in a top-level `val`'s initializer, its temporary. In a closure, a place rooted at a capture local is what the closure captures and is allowed (D8). Otherwise it is `E5008`.
- **An input of an argument at a tied position** (D2): allowed. In a closure, the input of a capture: allowed.
- **Any other input**: `E5009`, with the reason the parameter is not tied: another parameter of a method (rule 2), a function-typed parameter (D9), or, in a default, an untied earlier argument (D10).

Writes of the return place, rather than `Return`, give the error the returned expression's span (each `return e` and each branch of an expression body writes `_0` once on its path), and nothing is written to `_0` afterwards, since nothing borrows the return place. Each write reports at most one error: `E5008` for its earliest owned loan if any, else `E5009` for its first untied input by argument order. `Liveness` still does not use the return place, so a returned borrow of a local is never also `E5006`. Rejected: making the return place live at `Return`, which would report `E5006` "does not live long enough" for every returned local with no way to say which rule applies; checking at `Return`, which loses the returned expression's span.

### D6. Codes and messages (autonomous)

The next free codes: `E5008` `DanglingBorrowReturn`, named by `docs/architecture.md`, and `E5009` `UntiedBorrowReturn`, the "own code" of rule 2, which also covers the other ways a returned borrow derives from an input its rule does not tie the result to (a function-typed parameter, an untied earlier argument of a default), since the fix is the same: return a borrow of a tied input, or change the signature. `BorrowErrorKind` gains `DanglingBorrowReturn { borrowed: Borrowed, rule: Rule }` (`Borrowed::Local(name)`, `Parameter(name)`, `Temporary`) and `UntiedBorrowReturn { parameter: Option<String>, reason: UntiedReason }` (`NotReceiver { method }`, `FunctionParameter`, `DefaultByValue { default }`, `DefaultMutable { default }`).

| Code | Title | Primary label (at the returned value) | Related, notes, helps |
|---|---|---|---|
| `E5008` | "cannot return a borrow of local 'n'"; "... of parameter 's'"; "... of a temporary value" | by owner, as `E5007`: "returned here", "assigned to a top-level value here", "returned as a default value here" | at the borrow when its span differs from the primary's: "'n' is borrowed here" or "a temporary value is borrowed here"; under rule 4 the note "'f' takes no borrowed input, so it can only return borrows of top-level values (elision rule 4)"; helps "return the value itself instead of a borrow of it" (local or temporary), "take 's' as a reference parameter ('&String') so that the result can borrow the caller's value" (parameter), "declare the value as a top-level 'val' of its own and borrow that" (a top-level `val`'s temporary) |
| `E5009` | "method 'other' cannot return a borrow derived from parameter 'o'"; "cannot return a borrow obtained from function-typed parameter 'h'"; "default value of 'b' cannot borrow parameter 'a'"; "default value of 'n' cannot borrow from 'this', which the call borrows mutably" | as `E5008` | at the parameter's declaration: "'o' is a parameter here"; notes: "a method's returned borrow is tied to its receiver only (elision rule 2), so callers keep only the receiver borrowed"; "a function-typed parameter is not a borrowed input, so callers do not keep borrowed what the function passed for it borrows"; "the call passes 'a' itself, so a default can only borrow what a shared reference parameter refers to"; "the call passes 'this' as a mutable borrow beside the default, which would alias it"; helps: "return a borrow derived from the receiver, or declare a function taking the receiver as a reference parameter, whose result is tied to every reference parameter (elision rule 3)" for rule 2, "declare 'a' as a reference parameter ('&String')" for a by-value earlier argument, "pass 'n' explicitly at each call, or give it a default that does not borrow 'this'" for a mutable one |

`this` is named `'this'`; a closure is named "the closure" in rule notes. The wording follows rustc's E0515 ("cannot return reference to local variable") with Prism's "borrow". Rejected: one code for every rejected return (the dangling case's fix, returning an owned value, differs from the untied one's); a code per `E5009` reason (they share their fix); rustc's "lifetime may not live long enough", which names lifetimes Prism has no syntax for.

### D7. Borrows of top-level values hold nothing (user for "valid everywhere"; rejecting borrowed temporaries in initializers autonomous)

A borrow of a top-level `val` is a constant (`Constant::TopLevelValue`) that carries no loans, and a reborrow of it (`&(*_1)`) is a loan through a reference whose root holds nothing (D5), so it may be returned under any rule, including rule 4, and a caller's result built from it keeps nothing borrowed. What a top-level `val`'s initializer returns is checked like a function with no input borrows: `val REF: &Int = &LIMIT` passes, and `val R: &Int = &1` and `val S: &String = &make()`, which today hand every reader a borrow of a temporary that ended with the initializer, are `E5008`. Rejected: promoting constant temporaries to static storage as Rust does (`&1` as `&'static`), which needs constant evaluation the compiler does not have; it remains possible later and would only accept more.

### D8. Function types elide by rules 1, 3 and 4; a closure may return borrows of its captures (autonomous; settles the closure question of D9 of `borrowck-exclusivity`)

A function type has no receiver, so a call of a function value is tied to every input-borrow argument (rules 1, 3, 4) and to the function value's own items, which the caller already assumes (`borrowck-exclusivity` D9). A function value is either a closure or a top-level function (`Constant::Function`); an extension cannot be one (method references are `E4006`), so a function value's rule never is rule 2 and a function's own rule always matches its type's. A closure body may therefore return borrows derived from its reference parameters (their inputs, D4) and from what it captures: places rooted at a capture local, and borrows through a captured reference, whose loans the closure value holds at the caller (D4 of `borrowck-exclusivity`). So `val g = { &t }; val r = g(); t = "b"; greet(r)` is `E5004`, since `r` holds the closure's borrow of `t`, and the closure is accepted. A borrow of the closure's own local or temporary is `E5008`.

Rejected: forbidding closures from returning borrows of their captures, which would reject getters such as `{ &u.name }` that the caller side already handles soundly, and would make the caller's "function value's items" rule an unneeded over-approximation.

### D9. A function-typed parameter's result cannot be returned (follows from the user's definition of input borrows)

`docs/architecture.md` says function-typed parameters are not input borrows, so a caller does not keep a closure it passes borrowed beyond the call, and the closure may have returned a borrow of what it captures. A returned value holding the input of a function-typed parameter (`fun call(h: () -> &Int): &Int = h()`) is therefore `E5009`, and so is `fun apply(g: (&Int) -> &Int, v: &Int): &Int = g(v)`, whose result holds `g`'s input as well as `v`'s. Writing such a result through a `&mut R` parameter stays accepted, since the caller's `call_writes` already gives what the slot points at the closure's items. Counting function-typed parameters whose result type contains a reference as input borrows would accept `apply`, at the cost of keeping every closure passed to such a function borrowed while the result is used; it is a possible relaxation for `ROADMAP.md`, since it changes the user's definition.

### D10. Default values borrow only through shared references, and callers keep exactly that (autonomous; settles the default-generator rule and the `E5005` diagnostic of the `borrowck-exclusivity` review)

A default's generator receives `&P` for each earlier argument and, for a reference parameter, the `&mut R?` slot. Its result may hold:

- the slot's input (a borrow of the temporary it stores there, which lives until the end of the caller's statement);
- the input of an earlier argument whose `P` is a shared input borrow (`&T`, `(&T)?`, a `fun` receiver), through which it may return what that argument refers to (`fun User.hi(n: &String = name)`, `fun f(a: &User, n: &String = a.name)`);
- nothing else. Borrowing an earlier argument itself, a loan of `(*_i)` not through a further reference, is the caller's temporary, which the call takes: a by-value argument (`fun m(a: String, b: &String = a)`, `fun k(a: Int, b: &Int = a)`, a `consuming fun` receiver) or, rarely, a reference argument borrowed as `&&T`. Deriving from a `&mut` earlier argument or a `mut fun` receiver would alias the mutable borrow the call passes beside it (`mut fun User.rename(n: &String = name)`), and from a function-typed one is D9. Each is `E5009` at the default's declaration.

At the call site, a `Callee::Default` call's result keeps the items of the slot and of each tied earlier argument, minus the generator's own shared `Default`-origin borrows of the argument temporaries: what the receiver or reference argument borrows, not the temporary holding it. So `val h = u.hi(); u.name = "b"; greet(h)` is `E5004` and `val h = u.hi(); greet(h)` passes, where today the result keeps the receiver's temporary and is `E5006`; and `u.rename()` no longer reports `E5005` at the receiver, because the default's declaration is rejected instead. The rule treats `Copy` by-value arguments like the others: the default would point at the caller's copy, not at the callee's parameter, and a result tied to it could outlive the statement.

Rejected: keeping the generator's borrow of the argument temporaries at call sites (rejects returning a default's borrow from a method); accepting `Copy` by-value earlier arguments; naming the receiver in the `E5005` at the call (every call would still fail, so the declaration is the place to report it); rejecting the defaults in the type checker, which cannot see what a default's borrow derives from through calls (`n: &String = pick(&a, &b)`).

### D11. Bodies may store input borrows through inputs; by-value parameters end at the return (autonomous; settles the storing question; the soundness gap found while proposing)

A call is already assumed to write the items of its other operands through each `&mut R` argument (`call_writes`), and a closure the items of its other captures into each variable it captures mutably (`closure_writes`), so storing a borrow derived from an input there (`fun store(slot: &mut &Int, v: &Int) { *slot = v }`) is accepted: the caller keeps `v`'s borrows in what `slot` points at. What the callee must not store there is a borrow of memory it owns. Locals and temporaries already end before the return while those places are live at it (`returned_locals`), which is `E5006`; by-value arguments did not end when their type needs no drop. Each `Return` now also ends the storage of every argument whose type needs no drop (an access of kind `StorageDead` in `conflicts::location_events`, before the return's uses of the returned locals, so those uses keep the loans in scope); an argument that needs a drop ends at its elaborated drop, or by being moved. So `fun keep(slot: &mut &Int, v: Int) { *slot = &v }` and `run2 { x -> r = &x; 1 }` are `E5006`. Generator arguments are references and capture locals are not arguments, so neither is affected. Rejected: forbidding stores of input borrows through other inputs (they are sound and the caller models them); a separate code for a stored borrow of a parameter (it is a value that ends while borrowed, `E5006`'s case).

### D12. `E5006` labels parameters at their declaration (autonomous; settles the label item of the `borrowck-exclusivity` review)

The drop of an argument and the `Return` that uses a returned local carry the body's span, so today both labels of `fun store(slot: &mut &String, v: String) { *slot = &v }` point at the whole signature. When the value that ends is an argument, the related label is at the parameter's declaration, "'v' is dropped when the function returns, while still borrowed" ("when the closure returns" in a closure). When the later use is a `Return`'s use of an argument (a `&mut R` parameter the caller sees), it is at that parameter, "borrow later used by the caller through 'slot'"; a capture local's later use stays labelled at the closure. Rejected: labelling the closing brace of the body, which an expression body lacks.

### D13. A `(&mut T)?` lent as a `(&mut T)?` is reborrowed (orchestrator)

Where the typed tree reads a `(&mut T)?` place in place as a value (an argument of a `(&mut T)?` parameter), MIR lowering reads its tag and, on the present edge, passes `wrap &mut (*(place as payload))`, a mutable `Argument`-origin borrow of the payload's referent, and on the absent edge `null`, without moving the place, mirroring how a `(&mut T)?` lent as a `(&T)?` is reborrowed (`mir-lowering` D12) and how a `&mut` binding passed as an argument is (`references`). So `edit(m); edit(m)` leaves `m` usable, as the `references` rule for `&mut` bindings says, and the borrow checker sees a mutable loan of the referent that ends with the call; `both(m, m)` is `E5003`. `optional_borrow` in `build/comparison.rs` gains the borrow's mutability, and `Builder::operand` (`build/place.rs`) calls the new `lower_optional_reborrow` for a non-`Copy` place of type `(&mut T)?` instead of panicking. The typed tree is unchanged, since its spec already reads such a place in place, and a plain-fix-style regression fixture runs real source. Rejected: moving the value (the binding would become unusable after one call, unlike a `&mut` binding); a typed-tree coercion node for the same type, which would add a coercion kind for one case.

### D14. Pipeline (follows from D16 of `borrowck-exclusivity`)

`check_file` runs `returns::check_returns` on each elaborated body after the escape and conflict checks, with the same borrow set and loan flow, and sorts all errors as today. A file with move errors still stops before; returned borrows are checked on elaborated MIR because the loan flow is. The crate documentation of `lib.rs` describes the callee's half in place of the D21 guidance of `borrowck-exclusivity`.

### D15. Programs that relied on unchecked returns (follows from the rules)

Every passing fixture and example was written before returned borrows were checked. During implementation, each `// expect: ok` case or example that the checker now rejects is either a scenario whose outcome this change decides, or incidental and rewritten without changing what it exercises, and is listed in the implementation notes below. Known: the form `fun m(a: String, b: &String = a): Int = 1` of `ownership/moving-an-earlier-parameter-in-a-default-value--clone-and-copy`, not part of that scenario's text, is dangling under D10; it is removed there and becomes the fixture of "A default borrowing an earlier by-value parameter". Every other fixture that returns a borrow follows its rule (checked while proposing).

### D16. Fixtures and tests (autonomous)

- Every scenario of the delta specs gets a fixture under its spec's directory, one case per form: the cross-file scenario as a two-file workspace (`// entry: app/main.prism`), as `compilation-and-abi/borrow-checking-independent-of-dependency-bodies` does; the `mir` scenarios hold `mir.snap`.
- Unit tests in `prism-borrowck`: `elision.rs` for each row of D2's table; `borrows.rs` for the inputs of reference arguments; `loan_flow.rs` for rule 2 at call sites and the default narrowing; `returns.rs`, through `crates/prism-borrowck/tests/elision_tests.rs` lowering small files from source as `exclusivity_tests.rs` does, for each kind, reason and label.
- `crates/prism-cli/src/diagnostics.rs` tests render the two label scenarios and "Dropped parameter labels" exactly.

## Risks / Trade-offs

- [Rule 2 is strict on bodies: a method that needs its result tied to another argument must become a function] → that is the user's rule; the help says how, and the result is then tied to every input (rule 3).
- [Function-typed parameters' results cannot be returned (`apply`)] → sound and rare; a `ROADMAP.md` relaxation (D9).
- [The signature decides, so a method returning a borrow of a top-level value still keeps its receiver borrowed] → the cost of deciding from signatures alone, which the compilation-avoidance invariant needs: a body edit must not change what callers see.
- [Defaults that borrowed a by-value or `&mut` earlier parameter now fail] → each was unsound or failed at every call; the help suggests a reference parameter.
- [More items per body (an input per loan-carrying argument)] → one bit per loan-carrying local and argument; bodies are small, and the dense-bit-set item in `ROADMAP.md` covers growth.

## Migration Plan

No external users. Programs returning dangling or untied borrows, defaults borrowing by-value or `&mut` earlier parameters, and bodies storing borrows of by-value parameters through their inputs now fail `prism check`; method calls whose results kept other arguments borrowed, and results of defaults that borrowed through a receiver, are accepted more often. `BorrowSet::inputs` grows, `BorrowErrorKind` gains two variants, and `prism-mir` gains two helper methods and no data. Rollback is a revert.

## Open Questions

- Whether a constant temporary should be promoted to static storage (`val R: &Int = &1`), once constant evaluation exists (D7).
