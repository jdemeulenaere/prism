# Design: closures

## Context

See `proposal.md` for motivation. This is item 22 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17. Several earlier changes left questions for it: `typed-tree` D6 (function-type subtyping that changes a representation), `codegen-core` D5 (`#closure<i>` symbols), `codegen-aggregates` D8 (function types have no layout), `mir-lowering` and `borrowck-exclusivity` (capture locals and capture modes recorded for closure conversion), `monomorphization` D12 (closures inside generic bodies) and `heap-boxes` D11 (boxing a capturing closure). The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

**Baseline.** This change was proposed while `payload-patterns` was being implemented and `heap-boxes`, `string-templates` and `monomorphization` were proposed but not implemented. All four land before it. The `code-generation` delta modifies "Tier 1 Compiles Elaborated MIR in Memory", "Constructs the Backend Lacks Are Rejected Before Code Generation" (which `heap-boxes` adds) and "Cranelift IR Dump" as `monomorphization` leaves them, and "Layout Dump" as the main spec has it, so `openspec validate --strict` reports, as information, that one header is not in the main spec yet. It removes "Aggregates Holding Heap or Function Values Are Rejected", which commit `8e5cc47` split out of "Unsupported Constructs Are Rejected Before Code Generation" after `heap-boxes` was proposed: `heap-boxes` must narrow it to function values (or fold it into its "Drops of Values That Own Nothing Do Nothing"), and whatever requirement still rejects aggregates holding function values when this change is implemented is removed by it. Closure symbols are an ADDED requirement, so "Symbols Derive from Declaration Paths", which `heap-boxes`, `monomorphization` and that commit all touch, is left alone. When implementation starts, each MODIFIED and REMOVED block must be compared with the archived text again, as `monomorphization` asks.

What exists today:

- **Typed tree** (`crates/prism-typed-tree`). `ExpressionKind::Closure { parameters, body, is_compose, captures }` records each closure's captured bindings in first-use order. A function name read as a value is `FunctionReference(path)`, a payload variant read as a value `VariantConstructor`, and either may be wrapped in `Coerce { kind: Subtype }` when the expected function type differs (`val h: (Int) -> Unit = g` for `g: (Int?) -> Unit`). Function types are contravariant in their parameters and covariant in their results through every subtyping step (`is_assignable_to` in `crates/prism-typechecker/src/types.rs`), including `T` to `T?` and `Nothing` to `T`, which change a representation.
- **MIR** (`crates/prism-mir`). Each closure is its own body (`BodyOwner::Closure { parent, index }`, named `main::f#closure0`), whose capture locals follow its arguments (`LocalKind::Capture`), each standing for the captured binding itself: it has the binding's type, and it is never moved, marked or dropped. The parent builds the value with `Rvalue::Closure { body, captures }` from the places of the captured bindings, which are a local, a generator's `(*_i)`, or the parent closure's own capture local (`build/closure.rs`). A function name or variant constructor as a value is `Operand::Constant(Constant::Function(path))` or `Constant::VariantConstructor` (`build/place.rs`). A call of a function value is `Callee::Value(place, span)`, the place read in place. Function types are not `Copy`, so function values are moved and dropped (`needs_drop`).
- **Borrow checker** (`crates/prism-borrowck`). After elaboration, `captures::infer_capture_modes` gives each closure body one `CaptureMode` (`Shared` or `Mutable`) per capture local, in `Body::capture_modes`. The loan flow gives a closure value the loans of its captures, which last while the value is live, so captured places are never moved, reassigned or freed meanwhile (`E5003` to `E5006`). A capturing closure or function-typed parameter cannot escape (`E5007`, `escapes.rs`): a function value that holds a capture's loan or a function input may not be returned, stored in a field or payload, written through a reference, assigned to a captured variable or passed to a generic slot. A closure without captures may do all of these.
- **Backend** (`crates/prism-codegen`). `is_compiled` skips closure bodies and generic bodies. `classify` (`supported.rs`) names `Type::Fn` "function values and closures", so any body holding a function value, any `Rvalue::Closure`, any function constant and any struct with a function field is `E6001`. `layout.rs` gives `Type::Fn` no layout, `representation` none, and `lower.rs` returns an internal error for `Callee::Value`. `glue.rs::owns_storage` already counts function types as owning storage. Every call loads its callee's function-table slot (`<symbol>#slot`, `UnitContext::slot`) and calls it indirectly, so `hot-reload` can repoint slots.
- **Instances** (`monomorphization`). A generic record holds the closures lowered inside the generic's bodies, but `instantiate` and instance units do not compile them (its D12).

## Goals / Non-Goals

**Goals:**
- Every closure and function value that the borrow checker accepts today runs in Tier 1: closures passed to functions, held in locals, written into captured variables, nested, in default values, in generic bodies, and without captures in fields, nullables and top-level values. Function names and variant constructors as values run too. Every fixture runs under `--check-allocations`.
- One representation for every function value, whatever made it, that `escaping-closures` can extend with owned environments without changing a type's layout or a call's code.
- Hot reload reaches closures: a closure's code has a function-table slot, and a stored function value calls through it.
- Building and calling a closure of this change allocates nothing.

**Non-Goals:**
- Escaping closures, capture by move, heap environments, an `escaping` marker, and changes to `E5007` (`escaping-closures`, D14).
- `Copy` or `Clone` function types; equality of function values.
- Adapters that convert arbitrary function values between function types of different representations (D8).
- Composable closures and calls of composable function values, which wait for `compose-transform` (D10).
- Optimizations: inlining a closure passed straight to a known function, a niche for `(() -> T)?`, sharing adapters. Tier 2 gets inlining from LLVM.

## Decisions

### D1. Two changes: closure conversion now, escaping closures next (autonomous)

The roadmap item covers both running today's closures and designing escaping, move-capturing ones. The two are different kinds of work. The first is a backend change with one small type-system rule (D8). The second changes the language: what a closure captures by move, how a parameter says it keeps a function value, what `E5007` becomes, and how `generics` D7 is relaxed. It also changes the borrow checker's pipeline and adds a heap environment. Together they would be one of the largest changes of the road, and its language decisions would wait behind backend work. So this change runs the closures the language has, with a representation chosen so that owned environments fit in it (D2, D3, D9), and a new item 23, `escaping-closures`, follows it before `collections`. The direction for that change is recorded here (D14), since D2 and D3 are shaped by it. Rejected: one change, which would be too large to review; escaping closures first, since they need a backend that runs closures at all.

### D2. A function value is a descriptor address and an environment address (autonomous)

Every value of a function type, composable or not, is 16 bytes with the alignment of a pointer: at offset 0 the address of the *descriptor* of the closure that made it (D3), never null, and at offset 8 the address of its *environment*, or null when the closure captures nothing. It is held in memory (`Representation::Memory`), so every rule for aggregates applies: a local is a stack slot, a parameter is the address of memory the caller owns, a result is written through the caller's address, and a move copies 16 bytes. A nullable function value is a tag byte followed by the value, 24 bytes, by the general nullable rule. A struct field of function type is 16 bytes, laid out like any other field.

One representation serves a closure with captures, one without, an adapter for a function name (D7), and in `escaping-closures` a closure owning a heap environment. So `(A) -> R` is one type and a call never asks where the value came from.

Rejected:
- **A bare code pointer**, as C and Rust's `fn` types are: it cannot carry captures, so closures and functions would need different types.
- **A code address and an environment**: a call could not go through the function table, so a function value stored before a reload would keep running old code, and there would be no place for the drop that `escaping-closures` needs.
- **Three words (slot address, environment, drop function)**: one load fewer per call, but every later entry (a clone function if function values ever become `Clone`, a size) would grow every function value. A descriptor grows instead.
- **A pointer to a boxed closure**, which allocates for every closure, against the zero-cost memory safety invariant.
- **The environment inline in the value**: the size of a function type would depend on what each closure captures.

### D3. Descriptors: static data per closure body (autonomous)

Each closure body has one descriptor, a data object of its code unit named `<closure symbol>#descriptor` (`main::main#closure0#descriptor`), two pointer-sized words:

1. the address of the function-table slot of the closure's code (D4), written as a data relocation to the imported data symbol `<closure symbol>#slot`, so the descriptor never holds a code address and a repointed slot is followed by every value made before;
2. the environment's drop entry: the address of a function that drops the environment, or null when dropping the value frees nothing. Every descriptor of this change has a null entry, because no closure of this change owns its environment (D5). `escaping-closures` fills it for closures that do.

A descriptor is per body, not per value: every value one closure builds shares it, and a closure body is built at exactly one `Rvalue::Closure`. Old descriptors stay valid across `hot-reload`, since they live in old modules that stay mapped (`runtime-strings-and-drops` D3) and name slots that never move.

Rejected: descriptors per creation site (the same thing, since each closure body has one site), per function type (a descriptor must name the closure's own slot), and in the function table (slots hold code addresses only).

### D4. A closure's code is a function with its own slot (autonomous)

Each non-generic closure body is compiled as a function of its file's code unit, named by its MIR identity: `<parent symbol>#closure<i>`, so `main::f#closure0`, `main::f#closure0#closure0` for a nested one and `main::f#default(p)#closure0` for one in a default value. That is the name `codegen-core` D5 reserved. It has a slot in the function table like every other function, which only descriptors refer to. Its machine signature is the signature of its MIR body with one pointer, the environment's address, after the result address when there is one and before the parameters: `(result?, environment, parameters...)`. It takes the environment even without captures (then null), so every call of a function value has the same shape.

The body checks the stack on entry, and a runtime error inside it is located in its own file, as for every compiled body. The closure index is stable as long as the closures before it in its parent are, which is what `hot-reload` keys a body by.

Rejected: the environment after the parameters, which gains nothing and moves with every parameter list, and the environment in a fixed register (Swift's context register), which the host C calling convention does not have.

### D5. Environments of borrowing closures: one pointer per capture, in the creating frame (autonomous)

Every closure the borrow checker accepts today captures by borrow (`Shared` or `Mutable`). Its environment holds, for each capture local in capture order, the address of the captured place, at offset 8 × index, so it is 8 bytes per capture. A closure without captures has no environment and a null environment address. Inside the closure body, a place rooted at capture local `k` is reached by loading word `k` of the environment and applying the place's projections to the address loaded. Capture locals are never moved, so no read moves out of the environment. Both modes are pointers: a `Mutable` capture writes through its pointer, a `Shared` one only reads.

The storage is a stack slot of the function that builds the closure, one per closure body built in that function, written each time the `Rvalue::Closure` runs. A frame-lived slot is sound for borrowing closures. A closure value never outlives the frame that built it, since a capturing closure cannot escape (`E5007`), and a capture's loan keeps the captured place alive while the value is live (`E5006`). Within one frame, every run of the same `Rvalue::Closure` writes the same addresses. A captured local lives in a stack slot of its own for the whole frame, a generator's `(*_i)` is the same argument throughout, and a parent closure's capture is the same environment word throughout one activation. So rebuilding a closure in a loop rewrites the slot with the values it already holds, and a value from an earlier iteration still reads correct addresses. A binding declared inside the loop body ends at the end of each iteration, and its loans keep any closure that captures it from being used after that. A captured place must have an address: the backend keeps every local that a closure captures in memory, as it keeps every borrowed local (`borrowed_locals` in `lower.rs`).

Rejected:
- **Copying shared `Copy` captures into the environment**, which would save one load per read. A closure whose type takes no borrowed input may return a borrow of what it captures (`borrowck-elision` D8), and that borrow is tied to the closure's loans of the captured bindings. A copy in the environment would make it point at the environment instead, which the closure's value, not the binding, keeps alive.
- **The environment in an explicit MIR local with storage markers.** It needs an environment type in the type table and changes the MIR every pass reads, for storage that D5 shows can live as long as the frame.
- **A heap environment**, which allocates for every `run { ... }`, against the zero-cost memory safety invariant. Escaping closures need one (D14), non-escaping ones never do.

### D6. Calling a function value goes through its descriptor's slot

A `Callee::Value(place)` call reads the function value in place. It loads the descriptor address at offset 0, the slot address from the descriptor's first word, then the code from the slot, and calls it indirectly with the signature of D4, derived from the call's parameter types and result: the result address when the result is in memory, the environment address (offset 8 of the value), then the arguments as for any call (`codegen-aggregates` D10). That is three dependent loads, one more than a call of a named function, the price of reaching reloaded code from a stored value. The call keeps every argument rule of named calls, including copying a `Copy` operand into a fresh slot and passing a moved operand's own place.

### D7. Function names and variant constructors as values are adapter closures (autonomous)

A function's code takes no environment, and a variant constructor has no code at all, so neither can be a descriptor's target as it is. MIR lowering therefore turns each `FunctionReference(path)` and `VariantConstructor` read as a value, together with any `Coerce { Subtype }` directly around it, into an *adapter*: a closure body without captures, lowered in place of the reference as the next closure of its parent body (`BodyOwner::Closure`). Its arguments have the parameter types of the value's function type, the expected one when a `Subtype` wraps the reference. Its body calls the function, or builds the variant, with each argument converted to the declared parameter type, and writes the result converted to the value's result type. Each conversion is one a value conversion already lowers: `WrapNullable` for `T` to `T?`, `Subtype` for `&mut T` to `&T` and for a function-typed argument or result that converts in place (D8), and `Unreachable` after a call returning `Nothing`. The parent builds it with `Rvalue::Closure { body, captures: [] }`. `Constant::Function` and `Constant::VariantConstructor` leave MIR, so the backend has one kind of function value.

An adapter is borrow-checked like any closure, and always passes. A function used as a value cannot be an extension, so elision rule 2 never applies to it, and rules 1, 3 and 4 of a function and of a function type tie the result to the same inputs, so its call of the function is tied exactly as its own result must be. Adapters are not deduplicated: `apply(double)` written twice makes two tiny bodies. Their code calls the function through its slot, so they never need recompiling when the function changes.

Rejected:
- **An entry point with an environment parameter for every top-level function**, generated in the defining file's unit with a slot of its own (`lib/math::double#value`). It doubles the functions of every file, since the defining file cannot know which functions importers use as values, and still needs a per-site conversion for `(Int?) -> Unit` used as `(Int) -> Unit`.
- **Every function taking an environment**, so a function's slot could be a descriptor's target directly. It costs a register at every call of every function, for the few used as values.
- **Adapters generated by the backend per code unit**, like glue. They would need slots outside the symbol scheme, and MIR, the borrow checker and the dumps would not see them, while a MIR closure needs nothing new.

### D8. Function values keep their representation when converted (autonomous, BREAKING)

`typed-tree` D6 left a risk: the subtyping steps of function types through `T` to `T?` and `Nothing` to `T` are `Subtype` coercions that change a representation. `(Int?) -> Unit` given for `(Int) -> Unit` must receive an `Int` (one machine word) where its code reads an `Int?` (an address of a tagged value). `() -> Int` given for `() -> Int?` returns a word where the caller expects a result written through an address. A conversion must therefore build new code, an adapter, around the value it converts.

For a named function or a variant constructor, D7 builds that adapter at no cost, and a closure literal is checked against its expected type directly, so it never needs one. For any other function value, a local, a parameter, a field or a call's result, the adapter would have to hold the converted value. Holding it by borrow makes a borrowing closure out of a temporary, which ends with the statement and so could not even initialize a `val`. Holding it by move needs an environment that outlives the statement, which means a heap allocation per conversion, hidden in what looks like an assignment. `references` made references invariant for the same reason, a conversion must not hide a copy (user, `reference-access`), and `heap-boxes` D8 made boxes invariant.

So a function value other than a named function or variant constructor converts to another function type only when that changes no representation: each parameter and the result are equal or convert by `&mut T` to `&T`, also inside a nullable and inside a function type, and compose flags still match. A named function or variant constructor converts its own parameters and results through any step, since its adapter does the conversion, except a parameter or result of function type: the adapter would have to convert that function value, so it converts in place too (`all(each)` for `each(f: (Int) -> Unit)` and `all(g: ((Int?) -> Unit) -> Unit)` is rejected). The relation is `Type::converts_in_place(from, to)` in `crates/prism-typed-tree/src/types.rs`, used by the type checker for this rule and by the backend's `is_lowered_subtype`. Any other conversion of such a value is the mismatch of its site (`E4002`, `E4004` or `E4005`), with help suggesting a closure that calls it (`{ x -> g(x) }`), which states the conversion and its capture. `each(show)`, `val h: (Int) -> Unit = show` and `run { 1 }` for a `() -> Int?` parameter are unaffected.

Rejected: adapters for every function value (a hidden allocation, or a temporary that dies too early); dropping representation-changing variance even for named functions (`each(show)` is ordinary Kotlin); keeping the rule and rejecting such conversions only in the backend (`E6001` for a program `prism check` accepts, forever).

### D9. Dropping a function value runs its descriptor's drop entry

A drop of a function value, and of a struct field, enum payload or nullable payload holding one, runs the drop glue of its function type, `<unit>#drop(<function type>)`, one per function type a unit drops, all with the same code: it loads the descriptor, loads the drop entry, and when the entry is not null calls it with the environment's address. Every entry of this change is null (D3), so the glue never calls anything yet. The drop is dynamic because a function-typed parameter or field can hold either kind of closure once `escaping-closures` lands, and the caller's code must not change then. `glue.rs::owns_storage` already says that function types own storage, so structs holding them already get glue that calls this one.

Function types stay neither `Copy` nor `Clone` (the `ownership` tables): a copy would duplicate a borrowing closure's mutable borrows, and an owning closure's environment can only be cloned if every capture can, which its type does not say (D14). So there is no clone glue for function types, and `clone()` of a function value stays `E4018`.

### D10. Composable closures wait for `compose-transform` (autonomous)

A closure body that is composable (checked against a `compose` function type) is reported as `E6001` "composable functions" at the closure, as a `compose fun` is at its declaration, because `compose-transform` will give composable code a composer parameter and group keys. Building such a closure is its own body's error, so the parent body reports nothing for it. The compose flag of a function type does not change its representation in this change. A value of a composable function type can only be made by a composable closure or adapter, so no program that runs holds one yet. Calling one happens only in composable bodies, which are already rejected. Rejected: compiling composable closures as plain functions now, which `compose-transform` would have to undo, and whose behaviour no scenario could pin.

### D11. Closures of generic bodies are instantiated with their parent

`monomorphization` D12 left this. When `instantiate` builds an instance of a generic body, it also instantiates every closure lowered inside that body, with the same type arguments, transitively for nested closures, and places them in the instance's unit after the instance body. The `Rvalue::Closure` of each one, and the `BodyOwner::Closure` parent of each, are renumbered to the instance unit's body ids. They are named `<instance symbol>#closure<i>` (`main::apply<Int>#closure0`) with descriptors `<instance symbol>#closure<i>#descriptor`, and `prism dump-mir --instances` and `dump-clif --instances` print them after their parent. A closure of a generic default value goes with that default's instance. Adapters (D7) inside a generic body are closures like any other and are instantiated the same way.

### D12. Closures in default values and top-level values

A closure lowered in a default value's generator is a closure of that generator body, `main::f#default(p)#closure0`. A closure without captures, or an adapter, may initialize a top-level `val` (`val SEVEN: () -> Int = { 7 }`, `val TWICE: (Int) -> Int = double`): the initializer builds it into the value's storage, and top-level values are never dropped (`runtime-strings-and-drops` D12), which is fine since its environment is null. Calls of such a value read it in place through the accessor's reference, `(*_t)`, as for any top-level value.

### D13. `E6001` after this change, and its fixtures (autonomous)

`Construct::FunctionValues` is removed. What `E6001` still names is composable functions, which composable closures now share (D10), values of unknown type, and conversions the backend does not lower, of which the type system now produces none (D8). The fixtures and support tests that `heap-boxes` and `monomorphization` moved onto function values move again:

- the forms `--closure` and `--function-parameter` of "Unsupported constructs in several forms" become a composable closure and a parameter of unknown type (`fun first(w: Widget): Int = 1`);
- "One error per body" uses parameters of unknown types;
- "A struct is rejected" becomes "A struct holding a composable closure is rejected";
- "An unsupported construct in a generic body" uses a composable closure;
- "Dumping a file the backend rejects" uses `compose fun Label() {}`;
- `every_row_of_the_table` and `generic_and_closure_bodies_are_not_checked` in `supported.rs` follow.

Rejected: keeping a fixture of each removed form that now runs under the old scenario name, which would make the scenario's THEN false.

### D14. Direction for `escaping-closures` (autonomous, recorded)

D2, D3, D5 and D9 are shaped by the second half of the item, so its direction is recorded here. `escaping-closures` decides it, and may revise it:

- **Capture by move is inferred from where the value goes, not written.** A closure whose value may reach an escape point (returned, stored in a field, payload or `Heap`, written through a reference, assigned to a captured variable, passed to a generic slot or to an `escaping` parameter) captures every binding by value: a `Copy` one is copied and any other moved into the environment when the closure is built, and `E5001` reports later uses of a moved one, labelled with the escape that made it move. Every other closure keeps capturing by inferred borrow. The borrow checker decides it: today's escape points become the places that switch a closure to capture by move, found by the loan flow on the MIR as lowered, before moves are checked. That keeps Kotlin's syntax, with no `move`, for `Button(onClick = { count.value += 1 })` and for `val h = { ... }; Button(onClick = h)`. Rejected: Rust's `move` marker (noise on nearly every compose handler), Swift's capture lists, and deciding by syntactic position in the type checker (it misses a closure bound to a local first).
- **A parameter that keeps a function value says so: `escaping`.** A contextual keyword before a parameter's function type, in declarations and in function types' parameter lists (`onClick: escaping () -> Unit`, as `compose` is written), recorded in the ABI. Inside the callee such a parameter holds no loans and may be stored or returned. At a call, passing a value that may hold borrows to it is `E5007`. Unmarked function-typed parameters stay lent, as today, so `forEach`, `run` and `remember`'s `calc` never move what their closures capture. A function type whose parameter is lent is assignable to one whose parameter is `escaping` (contravariance: a function that accepts any function value accepts an owned one), and both are represented alike. Rejected: owned by default (every `forEach { println(name) }` would move `name`), and inferring it from the callee's body (a body edit would change callers, against the deterministic hermetic builds invariant).
- **A closure that captures by move captures only `val`s and never writes its captures.** Capturing a `var` by value would silently diverge from Kotlin, where the closure sees later writes (`var n = 1; Button(onClick = { println(n) }); n = 2`). It is a new borrow-checker code, the next free one after `E5009`, with help to copy the value into a `val`. Since captured `val`s cannot be written, calling a function value never mutates its environment, so a function value needs no exclusive access to be called. Rejected: Rust's `FnMut` (calling would need `&mut`, and a shared call through `&(() -> Unit)` could write), and Java's "effectively final" over all closures, which would reject today's borrowing closures that write captures.
- **An escaping closure holds no borrows.** Capturing a reference, a borrowing closure or a lent parameter by value keeps their loans, and so is `E5007`, as is moving a non-`Copy` value out of something the closure cannot take, such as a parent closure's capture or a default value's input (with help suggesting a clone into a `val`). Liveness can keep ignoring drops (`borrowck-exclusivity` D5): no loan carrier owns an environment.
- **An owned environment is one exact-sized heap allocation**, captures laid out by value like a struct's fields, made through the runtime allocator when the closure is built. Its descriptor's drop entry (D3) is the closure's environment drop glue in the closure's unit: it drops the captures in capture order and frees the allocation. A closure whose captures are all zero-sized allocates nothing. A move-capturing closure returns no borrow of its captures (`E5008`), since its environment is memory it owns, which may be dropped while the result is live. Borrowing closures keep `borrowck-elision` D8.
- **Function types stay neither `Copy` nor `Clone`.** Remembering a closure by value (`remember { { ... } }` returning the stored closure) would need a clone that a function type cannot promise, so it waits for a design of cloneable function values (a clone entry in the descriptor, and a marker on the type). Closures returned by a `calc` and stored in state already work, since `remember<T>` and `mutableStateOf<T>` take type parameters that move-capturing closures may fill.
- **`E5007` narrows to function values that may hold borrows**: lent parameters and closures that capture references, borrowing closures or values they cannot move. `generics` D7's "passed to a generic parameter" and `heap-boxes` D11's `Heap({ ... })` become escape points that switch to capture by move, and the `ROADMAP.md` relaxation of `E5007` for a closure written through a reference into the body's own storage, stored in a local struct or used as a default value is done by the same rule.

### D15. Dumps

`prism dump-mir` shows adapters as closures (`closure main::main#closure0 []` and its body), and no function constants. `prism dump-clif` prints closure functions in body order, as MIR orders them, and the drop glue of function types with the other glue. Descriptors are data and are not printed, but the IR of a function building a closure names `<closure symbol>#descriptor`. `prism dump-layout` lays out function types and the structs, enums and nullables holding them, so `struct Task { val run: () -> Unit }` is no longer listed as waiting for a construct.

### D16. No new capability (autonomous)

Representation, closure code and glue extend `code-generation`; runtime meaning extends `execution`; adapters extend `mir`; the conversion rule extends `type-system`. Rejected: a `closures` capability, whose rules would be split from the backend contract, MIR forms and typing rules they belong with.

## Risks / Trade-offs

- [Three dependent loads per call of a function value] → the cost of reaching reloaded code from stored values, in Tier 1 only. Tier 2 can call known closures directly.
- [D8 rejects some conversions Kotlin accepts (`val g: (Int?) -> Unit = show; each(g)`)] → the help writes the closure that does it, which costs nothing. Adapters for arbitrary values can be added later if programs show the need, which only accepts more.
- [D5's soundness rests on borrowing closures never outliving their frame] → that is `E5007` today and stays true in `escaping-closures`, whose escaping closures get heap environments. The fixture "A closure built in an earlier iteration still runs" pins the loop case.
- [Adapters are not shared between uses] → a few instructions per use of a function as a value. A per-file cache keyed by function and type is a later option.
- [Modifying requirements of unarchived changes] → they land first (Baseline above).

## Migration Plan

No external users. Closures, function values and aggregates holding them stop being `E6001`, so the fixtures that pinned them run (D13). Conversions of function values that change a representation become type errors (D8): the repository has none outside new fixtures, and `prism check` on `examples/` must still pass. MIR snapshots holding function constants change to adapter closures. Rollback is a revert.

## Open Questions

- Whether adapters should be shared per file, per (function, function type), with a body owner of their own, if their count shows up in compile times.
- Whether nullable function values should use a null descriptor as `null` (16 bytes instead of 24), with the other niches of the `codegen-aggregates` item.
