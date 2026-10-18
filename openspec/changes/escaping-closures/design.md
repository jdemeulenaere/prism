# Design: escaping-closures

## Context

See `proposal.md` for motivation. This is item 23 of "Then: the road to Milestone 2" in `ROADMAP.md`, split out of item 22 by `closures` (D1), whose D14 records a direction for it: capture by move inferred from escapes, an `escaping` parameter marker, captures of `val`s only under a new code, escaping closures holding no borrows, exact-sized heap environments freed through the descriptor, and function types that stay neither `Copy` nor `Clone`. This change confirms most of it and revises three points: closures that outlive a captured binding also capture it by value (D1), the new code also covers captures a closure cannot take (D5), and small environments are stored inline (D9). After the user made fast compilation a primary criterion (2026-10-19), a fourth revision moved the decision from a flow-sensitive borrow-checker stage to a linear, flow-insensitive pass of the type checker (D1, D2, D5), marked **(revised for compile speed)**. The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

**Baseline.** This change was proposed while `payload-patterns` was being implemented and `heap-boxes`, `string-templates`, `monomorphization` and `closures` were proposed but not implemented. All of them land before it. The `references` delta modifies "Capturing Closures Do Not Escape" as `heap-boxes` leaves it (with boxes among the escapes), and the `code-generation` delta modifies "Function Values Are a Descriptor and an Environment" and "Each Closure Has a Descriptor", which `closures` adds, so `openspec validate --strict` reports those headers as not yet in the main specs. When implementation starts, each MODIFIED block must be compared with the archived text again and any drift reconciled first.

What exists, or will once `closures` lands:

- **Typed tree** (`crates/prism-typed-tree`). `ExpressionKind::Closure { parameters, body, is_compose, captures }` records each closure's captured bindings in first-use order. `Type::Fn { is_compose, params, ret }` has no per-parameter information. A function's signature is its symbol's `Type::Fn` (`SymbolKind::Function::fn_type` in `crates/prism-typechecker/src/scope.rs`), and its parameters are typed-tree `Local`s of kind `Parameter`.
- **MIR** (`crates/prism-mir`). Each closure is its own body; its capture locals stand for the captured bindings and are never moved, marked or dropped. The parent builds the value with `Rvalue::Closure { body, captures: Vec<Place> }`. Arguments of function type are moved into calls (function types are not `Copy`), and the callee drops them. `Body::capture_modes` is empty as lowered and holds `Shared` or `Mutable` per capture in elaborated MIR.
- **Borrow checker** (`crates/prism-borrowck`). Stage 1 checks moves on the MIR as lowered (`check.rs`, `move_paths.rs`, `uses.rs`) and elaborates drops; `captures::infer_capture_modes` then records modes; stage 2 runs the loan flow, the conflict check, the escape check (`escapes.rs`, `E5007`) and the return check (`returns.rs`, `E5008`/`E5009`). A function value escapes when it is written to the return place, used as an aggregate operand, written to a place with a projection or to a capture local, or passed for a parameter whose declared type has a type parameter outside a function type; it is `E5007` when it may hold a capture's loan or a function-typed input. `heap-boxes` D11 adds `Heap({ ... })` as an escape.
- **Type checker.** A closure moving a non-`Copy` capture is `E4017`, "captured variable 's', which the closure borrows" (`checker.rs`). That stays true of closures that own their captures: a closure may be called again, so it never gives a capture away.
- **Backend** (`closures`). A function value is a descriptor address and an environment address; each closure has a descriptor holding its code's slot and a drop entry, null for every closure of `closures`; a borrowing closure's environment is one pointer per capture in a frame-lived stack slot; dropping a function value calls the drop entry when it is not null (`<module>#drop(<function type>)`); the runtime allocates through `prism_rt_allocate(size, alignment)` and frees through `prism_rt_free(pointer, size, alignment)`.
- **Compose plan** (`docs/architecture.md` §5, written while this change was proposed). Event parameters are `escaping` function types (`onClick: escaping () -> Unit`), handlers are moved into the nodes that keep them (D22 there), a handler whose environment holds only `Copy` captures is a stable parameter compared by descriptor and captures (D9 there), content lambdas stay lent (D10 there), root content is `escaping compose () -> Unit`, and a closure whose captures changed gets a new slot on reload (D23 there). This change provides the language side of all of these.

## Goals / Non-Goals

**Goals:**
- Closures can be returned, stored in structs, enums, boxes and generic containers, passed to parameters that keep them, and kept past the scope of what they capture, with Kotlin's syntax and no annotation at the closure.
- Capture by value only where a capturing closure is rejected today, apart from two rare shapes of the flow-insensitive rule (D1), and no closure that can borrow pays for an allocation.
- Deciding captures costs time linear in each body, inside type checking, with no extra pass over MIR (D2).
- Zero GC and zero hidden ARC: each owned environment has one owner, is allocated once when the closure is built and freed once when it is dropped, which every executable fixture checks under `--check-allocations`.
- Compose handlers capturing one `Copy` handle allocate nothing.

**Non-Goals:**
- Lending function values to unmarked parameters instead of moving them (Open Questions).
- Mutable environments (Rust's `FnMut`), `FnOnce`-like closures that consume their captures, cloneable or `Copy` function values (D7, D11).
- Escaping closures that hold borrows tied by elision (`fun f(r: &Int): () -> Int = { *r }`), which stay `E5007` (D6).
- Inline environments for borrowing closures (`closures` D5 keeps them in the frame).
- Composable closures, which stay `E6001` until `compose-transform`. The compose plan (`docs/architecture.md` §5, D10) keeps content lambdas lent and makes only root content (`runApp`, `composition`) `escaping compose () -> Unit`, which D3 supports.

## Decisions

### D1. A closure captures by value where its value may reach an owning position or outlive a captured binding (autonomous, revises `closures` D14; revised for compile speed 2026-10-19)

Capture by value is inferred, never written. A closure *owns* a capture, copying a `Copy` binding and moving any other into its environment when it is built, in two cases, both read off the source:

1. **Its value may escape**: it is written at an *owning position*, or it is a source of an *escaping* binding. Then it owns every capture. An owning position is one whose value must be owned: the operand of a `return` or the result of a function, closure or default-value body; an argument, or a generic receiver, for an `escaping` parameter (D3), for a parameter whose declared type has a type parameter outside a function type (`generics` D7), or of `Heap(...)` (`heap-boxes` D11); a field argument of a struct constructor or a payload argument of an enum variant; and the value assigned to a field, through a reference, or to a variable that the enclosing closure captures. A position reaches into `if` and `when` branches, block results, both operands of `?:` and the implicit nullable wrap, as the expected type does. A local binding's *sources* are the closure literals and bindings at the top of its initializer and of every value assigned to it, through the same constructs. A binding is *escaping* when it is read at an owning position, when a closure that owns its captures captures it, or when it is a source of an escaping binding.
2. **It may outlive a captured binding**: it is a source, directly or through other bindings, of a binding declared outside the block that declares one of its captures, as in `var h = {}; if (c) { val s = "a"; h = { greet(s) } }; h()`. Then it owns that capture only, and keeps borrowing the others.

Every other capture stays an inferred borrow (`references`, "Closures Capture by Inferred Borrow"), and nothing allocates. Conflicts (`E5003` to `E5005`) never switch a capture: `val h = { greet(s) }; take(s); h()` stays `E5005`, since owning `s` would only make it `E5001`.

The rule is flow-insensitive: it asks where a closure *may* go anywhere in its body, not whether it does on some path. Every program in which a capturing closure reaches an owning position or outlives a captured binding was rejected before (`E5007`, `E5006`), so the rule only turns rejected programs into programs that run, except in two rare shapes where it owns although a flow-sensitive analysis (D2, rejected) would have borrowed: a `var` that holds a capturing closure and later receives another closure that escapes (`var h = { greet(s) }; h(); h = { 0 }; store(h)` moves `s` into the first closure), and a closure stored in an outer variable but used only inside the captured binding's block (`var h = { 0 }; if (c) { val s = "a"; h = { greet(s) }; h(); take(s) }`, now `E5001` at `take(s)`). Both are reported naming the closure and the reason it owns its captures (`E5001`, or `E4028` for a `var`, D5 and D15), and the fix is one `val` per closure, or a clone. In exchange the rule can be predicted from the source without reasoning about liveness, and costs no borrow-checker pass (D2).

The first case is `closures` D14. The second is new: without it a closure kept past its binding's block has no way to be written at all, since there is no syntax to ask for a move. Owning only the outlived capture, rather than all of them, keeps a `var` counter captured beside it borrowed and keeps other non-`Copy` bindings usable afterwards.

**Compile-time cost and Kotlin-likeness.** The decision is part of type checking and linear in the body (D2). It needs no syntax: `Button(onClick = { count.value += 1 })`, `val h = { ... }; Button(onClick = h)`, `return { ... }` and `Item(onSelect = { ... })` read exactly as in Kotlin.

Rejected:
- **Rust's `move { ... }`**, which every compose handler would need: Kotlin code would not read as Kotlin, and forgetting it gives an error whose fix is always the same keyword. It would make the decision free, but the flow-insensitive rule is nearly free too.
- **A marker on the binding** (`val h: escaping () -> Unit = { ... }`) for closures that reach an owning position through a local: also cheap, but compose code binds handlers to locals often enough that the marker would be noise, and the propagation it saves is linear.
- **Swift's capture lists** (`{ [s] in ... }`), the same noise with more syntax.
- **Owning only at the literal's own position**, without following bindings: it misses `val h = { ... }; Button(onClick = h)`, which compose code writes constantly.
- **A flow-sensitive borrow-checker stage** (this design's first version, revised for compile speed on 2026-10-19): exact, so that no accepted program changed, but it ran a forward dataflow over locals times closures, iterated to a fixed point over the body's closures, and a backward liveness, before the move check. One round costs O(blocks × locals × closures), which grows with the cube of a body's size, and the rounds grow with nested escaping closures, for a precision gain in the two rare shapes above only.
- **Owning every capture of a closure that outlives one binding.** Simpler to state, but it rejects `var` captures and moves bindings the closure could borrow.
- **Retrying a failed borrow check with owned captures**, which would make the decision depend on every other error in the body.

### D2. The type checker decides, in one linear pass after each body (autonomous, revised for compile speed 2026-10-19)

Whether a capture is owned changes the move check (the outer binding is moved) and drop elaboration (the parent no longer drops it), so it must be known before the borrow checker runs. The type checker decides it, and the typed tree records it, as the typed tree already records every move, copy and borrow (`typed-tree`):

- **Recording.** While checking a body (a function, a default value or a top-level initializer, with the closures nested in it), the checker builds a *flow graph* whose nodes are the body's closure literals and local bindings. A node read or written at an owning position (D1) is a root. A node at the top of the initializer of, or of a value assigned to, a binding `b` has a *source edge* to `b`. A binding captured by a closure has a *capture edge* from that closure. The owning position travels with the expected type of bidirectional checking, so recording costs a flag per checked expression and an edge per initializer, assignment and capture.
- **Deciding.** After the body: escaping nodes are those that reach a root, backwards along source edges and forwards along capture edges, by one graph search; the outermost block a node may be stored in is the outermost declaring block over the bindings it reaches along source edges, computed once over the strongly connected components (cycles come from `var` reassignments) in reverse topological order. A closure owns every capture when it is escaping, and otherwise each capture declared in a block that does not enclose that outermost block. A binding captured by value for the second reason becomes a root, and one more search covers the bindings it reaches. Each pass is O(nodes + edges), linear in the body, and bodies without closures skip them.
- **Typed tree.** Each capture of `ExpressionKind::Closure` becomes `Capture { binding, mode }`, with `CaptureMode::Borrowed`, `Moved` or `Copied` (for a `Copy` binding), and the closure records its *ownership reason*: the first owning position in source order that made it escape (its span and kind), or the assignment that stores it outside a captured binding's block. `prism dump-typed-tree` prints `captures [move s, copy n, borrow count]`.
- **MIR.** Lowering builds the closure with `Rvalue::Closure { body, captures: Vec<Capture> }`, where `Capture::Borrowed(Place)` is a borrowed capture's place and `Capture::Owned(Operand)` moves (`Operand::Move`) or copies (`Operand::Copy`) an owned one, and keeps the reason on the closure's body (`Body::ownership_reason`) for diagnostics. Stage 1 then sees an owned capture as an ordinary operand, so moves, drop elaboration and the loan flow need nothing else. After stage 1, `captures::infer_capture_modes` records `CaptureMode::Owned` for owned captures and infers `Shared` or `Mutable` for the borrowed ones, so `Body::capture_modes` still holds one mode per capture local in elaborated MIR, and `prism dump-mir --elaborated` prints `closure main::f#closure0 [move _1, copy _2, &_3]`.
- **`E4028`** (D5) is reported by the same pass, which knows whether a captured binding is a `var`, a capture of the enclosing closure or a default value's input.

Rejected: a borrow-checker stage 0 on the MIR as lowered, deciding by a "which closures may this local hold" dataflow and a liveness (this design's first version: super-linear, D1, and a second analysis of the whole body for a decision the source shows); deciding in MIR lowering (the same graph on a less convenient form, and the typed tree, which records moves, would not show it); deciding in stage 2 with the loan flow, which needs elaborated MIR whose drops depend on the decision.

### D3. `escaping` marks parameters that keep a function value (autonomous, confirms `closures` D14)

A parameter whose type is a function type or a nullable function type may be written `escaping`: `fun button(onClick: escaping () -> Unit): Button`, `fun later(f: escaping (() -> Unit)? = null)`, and in function types' parameter lists, `val register: (escaping () -> Unit) -> Unit`. It is a contextual keyword: a modifier only directly after a parameter's `:` or at the start of a function-type parameter, and only when followed by `(` or `compose`, so `escaping` stays usable as a name and even as a type name (`fun f(x: escaping)`). Before any other type, or in any other type position (`val h: escaping () -> Unit`), it is `E2001`, saying that `escaping` marks parameters of function type.

It is part of the function's type: `Type::Fn` gains `escaping: Vec<bool>`, one flag per parameter (in the AST, `TypeKind::Function` and `Param` gain the same), and the type is rendered `(escaping () -> Unit) -> Unit`. The ABI records it (`AbiType::Fn` and each signature parameter), its hash covers it, and its type strings show it, so making a parameter `escaping` re-checks importers: their calls are checked differently (the deterministic hermetic builds invariant). Flags come from the declaration, never from the body, so a body edit cannot change a caller.

A function type whose parameter is unmarked is assignable to one whose parameter is `escaping`, in contravariant position as usual: a function that accepts any function value accepts an owned one. The reverse is a mismatch (`E4002`, `E4004`, `E4005`). Both are represented and called alike, since both move the argument into the callee, so the step converts in place (`Type::converts_in_place`, `closures` D8). A closure literal checked against an expected type takes its parameters' flags from it.

Rejected:
- **Owned by default** (every function-typed parameter keeps its value): `items.forEach { println(name) }` would move `name`, and every `run`, `repeat` and `remember` would allocate.
- **Inferring it from the callee's body**: a body edit would change what callers may pass, against the deterministic hermetic builds invariant, and an imported function's ABI would have to carry a body fact.
- **A type qualifier valid anywhere** (`val h: escaping () -> Unit`): a local, field or result already owns its value, so the flag would mean nothing there.
- **Swift's attribute spelling `@escaping`**: Prism has no attribute syntax, and `compose` set the precedent of a bare contextual modifier.
- **Other names** (`owned`, `stored`): `escaping` says what the callee may do and is known from Swift.

### D4. Inside the callee, an `escaping` parameter holds no borrow; unmarked ones stay lent (autonomous)

An `escaping` parameter is an owned value like any by-value parameter: it may be stored, returned, boxed, passed to another `escaping` or generic parameter, or captured by an owning closure. Since callers may only pass values that hold no borrow (D6), the loan flow gives it no input (`LoanFlow::initialize_boundary` skips it), so nothing it reaches is tied to the caller. An unmarked function-typed parameter keeps its input, and so cannot escape (`E5007`, with help to declare it `escaping`). MIR records the flag on argument locals (`LocalDeclaration::escaping`) and on calls (`TerminatorKind::Call::escaping`, one flag per parameter, from the callee's function type), which is what the escape checks read.

Passing a function value to any parameter still moves it, as today: an unmarked parameter is a by-value parameter that the callee drops. `run(h); run(h)` is `E5001`; `run { h() }` lends `h` through a borrowing closure and allocates nothing. Lending function values to unmarked parameters, as Swift does, would accept more programs and is an open question (Open Questions), since it changes who drops the argument and how elision treats it.

### D5. Owned captures are `val`s the closure can take: `E4028` (autonomous, revises `closures` D14; revised for compile speed 2026-10-19)

A capture by value is `CannotCaptureByValue` (`E4028`, the next free type-checker code after `monomorphization`'s `E4027`, or the next free one when this lands) when it captures:

- **a `var`**, `Copy` or not. Kotlin closures see later writes to a captured `var` (`var n = 1; Button(onClick = { println(n) }); n = 2` prints `2` when clicked); an owned copy would silently print `1`. The help suggests copying it into a `val` first, and, in UI code, a state handle.
- **a non-`Copy` binding the closure's builder does not own**: a capture of the enclosing closure (which may be called again, and so cannot give its capture away) or an earlier parameter read in a default value (which only borrows it, `ownership`, "A Default Value Does Not Move Earlier Parameters"). The help suggests cloning it into a `val` first. A `Copy` one is copied.

The error labels the capture and the reason the closure owns it (D15). The type checker reports it from the pass of D2, which decides captures, and records that capture as borrowed, so the file stops before borrow checking as for any type error. The first version of this design gave it the borrow-checker code `E5010`, from its stage 0; with the decision in the type checker (revised for compile speed), the code takes the type checker's range, which the `diagnostics` spec ties to the stage that reports it.

`closures` D14 gave `var` captures the new code and the second kind to `E5007`. One code for both is clearer by fix, which is the catalog's rule: both are fixed by making a `val` the closure can take, while `E5007` is fixed by not capturing a borrow. Rejected: capturing a `var` by value (the silent divergence above); Java's "effectively final" for every closure, which would reject today's borrowing closures that write captures; boxing captured `var`s on the heap and sharing them, which is hidden reference counting or a hidden aliasing rule.

### D6. An escaping value holds no borrow: `E5007` narrows (autonomous, confirms `closures` D14)

At each escape position the escape check (`escapes.rs`) rejects a value whose type contains a function type when it may hold a borrow: a loan of any kind, or the input of an argument or capture whose type can carry loans other than an `escaping` function parameter. In practice that is a closure that owns a reference (`fun f(r: &Int): () -> Int = { *r }`), a borrowing closure or an unmarked function-typed parameter by value, or such a parameter passed on itself (`fun k(p: () -> Int): () -> Int = p`). The loan flow gives a closure value the items of its owned captures' operands, as it already does for borrowed ones. Closures without captures and closures owning only values that hold no borrow pass, which covers the `E5007` cases of today that were only conservative: closures written through references into the body's own storage, stored in local structs, used as default values, boxed and passed to generic parameters.

The escape check stays the safety net for every escape position: a capturing closure that reaches one by a way D1 does not follow still borrows, and is rejected here rather than accepted unsoundly.

A closure that owns its captures only because it outlives one (D1, case 2) does not escape, so it may hold borrows of what it still borrows or of references it owns, and their loans keep the borrowed values alive as for any closure (`E5006`).

Rejected: letting escaping closures carry borrows tied by elision (a returned closure tied to the inputs, like a returned reference): sound, but it needs lifetimes on function values that the zero-cost memory safety invariant's "no lifetime annotations" makes hard to explain, and no compose case needs it. It remains a possible relaxation.

### D7. Calling a function value borrows it, and environments never change (autonomous, confirms `closures` D14)

Owned captures are `val`s (D5), and a `val` is never written, written through a field, or passed as a `mut fun` receiver (`E4003`), so a closure's body never changes its environment. A call of a function value therefore reads it in place, as `closures` D6 lowers it, needs no exclusive access, and may go through a shared reference or a `val`: `b.onClick()` for `b: &Button` runs the handler. Calling never moves it: only passing or storing it does. A move-capturing closure's body still writes what it reaches through an owned `&mut` reference, which is not its environment.

Compose state lives in `State<T>` handles, which are `Copy` IDs into the slot table (the first-class compose invariant), so `count.value += 1` writes the slot table, not the captured handle. That sets a constraint for `state-handles`: writing a state's value must go through a call that takes the handle by value or by shared borrow, never as a `mut fun` receiver or a field assignment of the handle.

Rejected: Rust's `FnMut` (a mutable environment and calls through `&mut`), which would split function types into two families, make `() -> Unit` in a struct field ambiguous and force every UI tree walk to borrow handlers mutably; Rust's `FnOnce`, for which no compose case exists. A mutable environment could later be a distinct, explicitly marked function type, an addition rather than a change.

### D8. An owned environment is one exact-sized heap allocation, dropped through the descriptor (confirms `closures` D14)

A closure with at least one owned capture has an *owned environment*: one allocation of exactly its size and alignment through `prism_rt_allocate`, made when the closure is built, holding each owned capture by value and each borrowed capture as a pointer, laid out by the struct layout rule (fields by decreasing alignment, `codegen-aggregates`). Building the value moves or copies each owned capture's operand into its field and writes each borrowed capture's address into its pointer field. Inside the closure's body, a place rooted at an owned capture is reached at its field's offset from the environment address, and one rooted at a borrowed capture through the pointer loaded from its field.

The closure's descriptor (`closures` D3) points its drop entry at `<closure symbol>#drop-environment`, a function of the closure's code unit taking the environment address: it drops each owned capture that needs a drop, in capture order, through its type's drop glue, then frees the allocation with its size and alignment. The entry is a direct code address, not a slot: a value built before `hot-reload` keeps the drop function of the layout it was built with, from its old module, which stays mapped (`runtime-strings-and-drops` D3). A borrowing closure keeps a frame-lived environment and a null drop entry (`closures` D5). An owned environment is never kept in the creating frame, even for a closure that does not escape (D1, case 2): rebuilding a closure in a loop would overwrite captures that a value from an earlier iteration still owns.

Rejected: a reference-counted environment (hidden ARC); an arena per frame (an escaping closure outlives its frame); an environment owned by the slot table (only composable code has one).

### D9. A small owned environment is stored in the environment word (autonomous)

When an owned environment's layout fits in one pointer-sized word (size at most 8, alignment at most 8) and no capture needs a drop, its bytes are stored in the function value's environment word itself, the descriptor's drop entry is null, and building the closure allocates nothing. The closure's body spills the word it receives to a stack slot on entry and reads its captures from there as from any owned environment, so nothing else changes: callers, calls, drops and layouts of function types are as `closures` D2 defines them, since only the closure's own code and descriptor know how its environment is held. A closure that owns only zero-sized captures is the degenerate case.

This is the compose handler of the proposal: `{ count.value += 1 }` captures one `Copy` `State<Int>` handle, a 64-bit ID (`docs/architecture.md` §3, "Capturing State<T> handle in event closure"), so rebuilding it on every recomposition costs no allocation. Rejected: always allocating (an allocation and a free per handler per recomposition, for nothing); inline environments of two words, which would change the size of every function value; inlining borrowing environments with one capture too, which would change `closures` D5 for little gain (Open Questions).

### D10. A closure returns no borrow of what it owns (confirms `closures` D14)

In a closure's body, a borrow of an owned capture is memory the closure owns, so returning it is `DanglingBorrowReturn` (`E5008`), "cannot return a borrow of captured 's', which the closure owns", like a borrow of a local: callers tie a function value's result to its own borrows (`borrowck-elision` D8), and an owned environment may be dropped while the result is live. `returns.rs` treats a loan rooted at a capture as the capture's input only when the capture is borrowed. Borrowing captures keep `borrowck-elision` D8. Rejected: tying a call's result to the function value itself (sound, but a new elision case; a possible relaxation).

### D11. Function types stay neither `Copy` nor `Clone` (autonomous, confirms `closures` D9 and D14)

An owned environment can be cloned only when every capture can, which `() -> Unit` does not say, and a borrowing closure's mutable borrows cannot be duplicated. A type-level marker (a cloneable function type, with a clone entry in the descriptor) would be needed, and the planned compose engine needs none (`docs/architecture.md` §5): `remember` keeps `Copy` values only (its D8), so closures are not remembered; a handler is rebuilt on each composition and moved into the node that keeps it, which drops the one it replaces (its D22); and skipping compares a new handler with the kept one by descriptor and `Copy` captures, through an equality entry that `compose-transform` adds to descriptors (its D9), never by cloning. D9 makes that comparison a word compare for the common one-handle handler. Cloneable function values are a follow-up.

### D12. Generic slots and boxes accept owning closures; type arguments stay borrow-free

A generic parameter (`generics` D7) and `Heap(...)` (`heap-boxes` D11) are escape positions (D1), so `keep({ greet(s) })`, `Heap({ greet(s) })`, `Box({ greet(s) })` and, with `collections`, `list.add { ... }` build owning closures, and `List<() -> Unit>` holds handlers. `E5007` still rejects a value that may hold a borrow there, so a generic body may keep any value of a type parameter, as `generics` D7 requires. Type arguments stay borrow-free (`E4022`): `() -> Unit` contains no reference outside a function type. The planned `remember<T : Copy>(calc: () -> T)` (`docs/architecture.md` §5) keeps an unmarked `calc`, so `remember { mutableStateOf(0) }` borrows and allocates nothing.

### D13. No closure refers to itself, so owned environments form a tree (autonomous)

A closure cannot name the binding it initializes (`val f = { f() }` is `E4001`, as Kotlin rejects it), cannot hold a reference to itself in an owned environment (`E5007`), and recursion goes through named functions. Every owned environment is owned by exactly one value, which no environment reachable from it owns, so single-owner drops free every environment once and no cycle can leak: no weak references or cycle collection are needed. Rejected: Kotlin's local `fun` and recursive lambdas through a `lateinit var`, which need a self-reference that single ownership cannot express.

### D14. Liveness can keep ignoring drops

`borrowck-exclusivity` D5 left a risk: a closure that owns some captures and borrows others (D1, case 2) is a loan carrier whose drop runs code. That code drops owned captures through structural drop glue, which frees memory and never reads through a reference or runs user code, so no loan it carries is used by its drop. Liveness may keep ignoring drops until user-defined destructors exist, which must revisit it. Escaping closures carry no loans at all (D6).

### D15. Diagnostics

- **`E5001` after a capture by value** labels the closure "value moved into closure here" and adds a related location for the reason the type checker recorded (D2, `Body::ownership_reason` in MIR): "the closure owns its captures because it is returned here" (or "stored in a field here", "passed to an escaping parameter here", ...), or, for an outlived binding, "the closure is stored here, in a variable that outlives 's'". `UseKind::Captured` already words the primary label.
- **`E5007`** reads "a closure that captures the reference 'r' cannot escape", "... captures 'h', which borrows 's', ...", or "the parameter 'p' cannot escape", labelled at the escape as today, with notes that an escaping closure owns what it captures and a reference cannot be owned, and help to pass the value instead of a reference or to declare the parameter `escaping`. The note "escaping closures that capture by move are not designed yet" goes.
- **`E4028`** reads "cannot capture the variable 'n' by value" or "cannot move 's' into the closure", labelled at the capture inside the closure, with the reason as a related location, a note why (a `var`; captured by the enclosing closure; borrowed by the default value), and help: "copy it into a 'val' first: 'val m = n'", or "clone it into a 'val' first".
- **`E4017`** for a closure moving its capture reads "cannot move out of captured variable 's'", with a note that a closure may be called more than once, so it never gives away what it captures, whether it borrows or owns it. The type checker cannot know which.
- **`E5008`** for an owned capture: D10.

### D16. Closures of generic bodies decide once

The pass of D2 runs on a generic body like on any other, and its decisions do not depend on type arguments (a capture of a `T` is a move unless `T : Copy`), so the generic record carries them and every instance inherits them. Each instance's closures get their own environment layout, descriptor and `#drop-environment` function, in the instance's unit (`closures` D11).

### D17. Default values and top-level values

A closure built in a default value's generator and returned as the default escapes, so it owns its captures: `fun pick(a: Int, f: () -> Int = { a * 2 })` copies `a` and runs, where it was `E5007`; a non-`Copy` earlier parameter is `E4028` (D5). Top-level values are never captured, so a closure initializing a top-level `val` still captures nothing and has a null environment.

### D18. No new capability (autonomous)

Capture inference, `escaping` and `E5007` extend `references`; moves and `E4028` extend `ownership`; the typed-tree record of capture modes extends `typed-tree`; the syntax, typing, ABI and formatting of `escaping` extend `syntax`, `type-system`, `compilation-and-abi` and `formatting`; the MIR form extends `mir`; environments extend `code-generation`; runtime meaning extends `execution`. Rejected: a `closures` capability, whose rules would be split from the borrow-checking, backend and typing rules they belong with.

## Risks / Trade-offs

- [The inference is not visible in the source] → it is a rule about positions and blocks that the source shows (D1); the typed-tree and MIR dumps show every owned capture, and every error caused by one (`E5001`, `E4028`, `E5007`) names the closure and the reason it owns its captures.
- [The rule is flow-insensitive, so two rare shapes own where borrowing would do (D1)] → each error names the reason, and one `val` per closure or a clone fixes it; a flow-sensitive refinement could be added later and would only accept more programs.
- **Compile-time cost.** Deciding captures is a flag per checked expression, an edge per initializer, assignment and capture, and two or three linear graph passes per body with closures (D2); there is no borrow-checker stage. `escaping` flags cost one comparison per parameter in assignability and one byte per parameter in the ABI hash (D3). An owned environment adds one `#drop-environment` function per closure that allocates (D8), linear in the closures. The implementation adds an ignored timing test that checks a generated body of a few thousand closures and locals and reports the decision's share of `prism check` time.
- [An allocation per escaping closure built] → explicit in the sense of the zero-cost memory safety invariant: one owner, freed deterministically, never counted. D9 removes it for the common compose handler; `compose-transform` may memoize handlers whose captures are unchanged.
- [`hot-reload` repoints a closure's slot to code that may expect a different environment layout] → values built before keep their old drop function (D8), but would run new code on an old environment. The compose plan already requires that a closure whose captures changed gets a new slot (`docs/architecture.md` §5, D23); `hot-reload` must apply the same rule to every function value, recorded for that change.
- [Passing a function value to an unmarked parameter moves it] → `run { h() }` lends it at no cost; lending by default is an open question.
- [Modifying requirements of unarchived changes] → they land first (Baseline).

## Migration Plan

No external users. Programs that were `E5007` because a capturing closure escaped, and programs that were `E5006` because a closure outlived a captured binding, now pass and run, and their fixtures become running programs under the modified scenarios. Two rare shapes that passed now own a capture and may become `E5001` or `E4028` (D1): a `var` holding a capturing closure that later receives an escaping one, and a closure stored in an outer variable that is used only inside its captured binding's block; the repository has neither outside new fixtures. ABI hashes of signatures with function-typed parameters do not change unless a parameter is marked `escaping`. Rollback is a revert.

## Open Questions

- Whether unmarked function-typed parameters should borrow their argument rather than move it, so a stored handler can be passed to `forEach` twice; it would only accept more programs, but changes drop responsibility and needs an elision rule for it.
- Whether borrowing closures with one capture should keep its pointer in the environment word instead of a frame slot, like D9.
