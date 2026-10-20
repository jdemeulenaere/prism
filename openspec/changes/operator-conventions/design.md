# Design: operator-conventions

## Context

See `proposal.md` for motivation. This change is part **L1** of agreed item **A4** of the post–Milestone 2 review (`docs/milestone-2-review-notes.md`, "Agreed changes": "Language additions so the rest can be Prism: (L1) Kotlin-style `operator fun` conventions"), item 7 of "After Milestone 2: the review round" in `ROADMAP.md` ("Kotlin-style `operator fun` (`get`, `set`, `iterator`, `plus`, `compareTo`, `equals`, `contains`, `invoke`)"). The items after it rewrite the library in Prism: `drop-and-clone-hooks` (L2), `vararg` (L4), `library-in-prism` and `layout-in-prism`, which will want `Dp` and `TextUnit` arithmetic and ordering (`16.dp + 8.dp`, `padding * 2`, `coerceIn`), `Int.times(Dp)` as in Compose, equality of library types whose meaning is not their fields', indexing and iteration of list-like library types, and membership tests. The user is away: every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`; those taken in the review are **(user direction 2026-10-20)**. Compile speed is a primary criterion (AGENTS.md §5), so each decision states its compile-time cost.

**Baseline.** Proposed on top of `4c12f2e` ("Propose explicit-library"), with `path-captures` being archived and `owned-state` (item 4), `strong-skipping` (item 5) and `explicit-library` (item 6) proposed, all landing before this change. This packet is written for the state after them: no prelude, extensions resolved through the file and its imports only, built-in types' members everywhere through the member modules, the library under `library/prism/` (explicit-library D1 to D5). It touches none of their MODIFIED or ADDED blocks; its new requirements go to a new `operators` spec, and the blocks it modifies (`type-system` "Static Type Inference and Assignment Compatibility" and "Compound Assignment Typing", `collections` "Elements Are Places" and "For Loops Over Lists", `loops` "For Loops Over Ranges", `execution` "Equality of Structs, Enums and Nullables") are modified by no other open packet. The first task re-checks this.

**Baseline update (from `explicit-library`'s apply, not yet archived).** Names this packet may meet: the prelude is gone (`PRELUDE`, `PreludeScope`, `test_prelude()` and `TypeChecker::with_prelude` no longer exist); `MEMBER_MODULES` (`prism/core`, `prism/collections`) feed a `MemberScope` holding only the members of the built-in types, installed with `TypeChecker::with_members`; in-crate tests call `TypeChecker::with_library_imports()`, which ingests `/prism/core`, `/prism/collections` and `/prism/compose/runtime` as direct imports. A file's own declaration of an imported library name shadows it (no `E4010`; `explicit-library` D4 as corrected). A library type named in a signature without its import stays opaque, so fixtures naming library types must import their modules.

**Baseline update (apply, 2026-10-20).** `path-captures` (f6a6da7), `owned-state` (fd397f6), `strong-skipping` (c935cc8) and `explicit-library` (adf94de) are archived. Every MODIFIED block (`type-system` "Static Type Inference and Assignment Compatibility" and "Compound Assignment Typing", `collections` "Elements Are Places" and "For Loops Over Lists", `loops` "For Loops Over Ranges", `execution` "Equality of Structs, Enums and Nullables") was compared with the main text: each keeps every scenario of its main requirement and differs only by the sentences this change adds; no ADDED requirement exists in a main spec. `E4035` is free (`E4030` to `E4034` are taken, `E4033` retired); of the open packets, `drop-and-clone-hooks` builds on `E4035` being `InvalidOperatorDeclaration` and takes `E4036`, and `vararg` takes no `E40xx` code. Extension lookup is members first (the `MemberScope` of the member modules for built-in types, intrinsic members of library types where their module is imported), then the file's own extensions, then its imports' (`find_extension`, `find_extension_through_indirection`), with no prelude layer, so D3 needs none; `List<T>.contains` is the intrinsic catalog entry `ListContains` of `/prism/collections` (`crates/prism-typechecker/src/intrinsics.rs`, `library/prism/collections.prism`). A library type named without its import is now `E4001` (`explicit-library` D15), not opaque. `owned-state`: a state's value is a place, borrowed in every context and guarded at run time outside composable contexts, so an accessor or cursor call on `s.value` borrows it as any receiver does, and a `mut` accessor or `mut` `next` on it is a mutable borrow that `reject_state_mutation` reports in composable contexts (`E4031`). `strong-skipping`: kept arguments compare through the code generator's equality glue (function values by their identity words), so a type's `equals` reaches them through `expand` (D16) with nothing else to change. Every name the tasks use exists (`suggest_library_imports` is in `crates/prism-cli/src/library_index.rs`). `openspec validate operator-conventions --strict` is clean.

Current state (main specs and code):

- **Operators are built in.** `infer_binary_op` and `infer_unary_op` (`crates/prism-typechecker/src/checker.rs`) type arithmetic on `Int`/`Double` (and `String` `+`), ordering on `Int`/`Double`/`Char`, equality on every type but type parameters, and report `E4002` (`OperatorMismatch` or `TypeMismatch`) otherwise. `check_compound_assignment` types `t op= v` as `t = t op v` for the same operand types.
- **Indexing** (`infer_index`) accepts a `List` (an `Index` place node) or a `String` (the built-in character call) and is `E4002` for anything else; `ExprKind::Index { target, index }` holds one index. Whether a place may be written is computed beside it (`infer_place` returns `(id, writable)`), and callers that write report `E4003`.
- **`for`** (`check_iterable`) accepts a `Range` (by value) or a `List` (borrowed for the loop, shared or mutable), recorded as `ForIteration::{Range, List}`; MIR walks a list by index.
- **Equality** is structural: the code generator expands `==` inline up to a budget of 32 comparisons and calls per-type equality glue (`<module>#equals(<type>)`, `crates/prism-codegen/src/equality.rs`) beyond it, for boxes and for list `==`, `contains` and `indexOf`; compose's kept values and state records compare through the same glue.
- **Extensions** live in a table keyed by receiver type constructor and name (`find_extension`, `ExtensionsOfName` by context), consulted after members (fields, built-in `clone()`/`unbox()`, intrinsic members); two extensions of one key and context in a file are `E4010`, as is one a member always shadows.
- **There is no `in` expression**: `in` appears only in `for` headers and `when` range patterns (`PatternKind::InRange`).
- **Structs cannot hold borrows** (`syntax`, "Struct Field Borrow Prohibition"), and there are no lifetime annotations (the zero-cost memory invariant).

## Goals / Non-Goals

**Goals:**
- A library type written in Prism gets Kotlin's operator syntax: arithmetic, ordering, equality, membership, indexing (as a place), calls and `for`, with Kotlin's names and meanings.
- Constant compile-time cost per operator use (one or two table lookups), no overload resolution, no new analysis.
- Operator uses are ordinary calls for MIR, the borrow checker and the backend: no new ownership rules, and every existing borrow and elision rule applies.
- Built-in operators keep their meaning and their speed; programs accepted today keep their meaning.
- `for` over a custom collection binds borrows, allocates nothing and needs no struct holding a borrow.

**Non-Goals:**
- `rangeTo`/`rangeUntil`, `plusAssign` and the other assignment operators, `unaryPlus`, `inc`/`dec`, `componentN`, `getValue`/`setValue`, `hashCode` (D2).
- Overloading an operator name by parameter type (D3), generic `equals` (D6), composable operators or operators with a context parameter (D5).
- `in`/`!in` patterns over containers in `when` (D8).
- Traits or interfaces; equality as a bound on type parameters (`==` on `T` stays `E4002`).
- Rewriting any library type to use operators: that is `library-in-prism` and `layout-in-prism`; this change only makes `List<T>.contains` an intrinsic operator (D8).

## Decisions

### D1. Kotlin-style `operator fun` conventions, resolved by name (user direction 2026-10-20)

The review agreed "Kotlin-style `operator fun` conventions" (A4, L1) so that the library can be ordinary Prism. An operator use is a call of a function with a conventional name on the receiver's type: `a + b` is `a.plus(b)`. The `operator` modifier is required, as in Kotlin, so a function that happens to be named `get`, `next` or `contains` never becomes an operator by accident, and the reader of a declaration sees which functions syntax reaches. Operator functions are extensions (Prism's only methods): declared in any file, found through the file and its imports (D3), callable by name.

**Compile-time cost.** One flag per function declaration; the lookups are D3's.

Rejected:
- **Traits or interfaces** (`impl Add for Vector2`, `: Comparable<T>`): a conformance system, coherence rules and bounds on generics, which `generics` defers until traits exist; the conventions need none of it, and a later trait can be satisfied by the same functions.
- **No modifier** (any function named `plus` is the operator): Kotlin requires the modifier for the reason above, and the signature checks (D5) need to know which functions claim a convention.

### D2. Which conventions, and what is deferred (autonomous)

In scope: `plus`, `minus`, `times`, `div`, `rem`, `unaryMinus`, `not`, `compareTo`, `equals`, `contains` (for `in` and `!in`), `get`, `set`, `invoke`, and `iterator`/`hasNext`/`next` for `for`. These are the ROADMAP's list plus the arithmetic and unary operators it abbreviates ("`plus`"), and they cover what items 8 to 11 need: `Dp`/`TextUnit`/`Offset` arithmetic and ordering, `Int.times(Dp)`, `!` on flag sets, membership in library collections, indexing and iterating list-like library types, and equality of types whose fields are not their identity.

Deferred, each recorded in `ROADMAP.md` at archive:
- **`rangeTo`/`rangeUntil`**: `..` and `..<` build the built-in `Range` of `Int`s, which is all the library uses; custom ranges need a range type per element type.
- **`plusAssign` and the other assignment operators**: `a += b` is `a = a + b` (D13); an in-place form matters for owning types, which `drop-and-clone-hooks` introduces, and would add Kotlin's ambiguity rule (both `plus` and `plusAssign` visible).
- **`unaryPlus`** (Prism has no unary `+`), **`inc`/`dec`** (`++` and `--` are rejected by design), **`componentN`** (destructuring is by field name), **`getValue`/`setValue`** (delegated properties, open item C3), **`hashCode`** (no hash maps yet).
- **Overloads by parameter type** (D3), **generic `equals`** (D6), **composable operators and operators with a context parameter** (D5), **`in` patterns over containers** (D8).

**Compile-time cost.** None.

Rejected:
- **Only what the library needs this round** (`plus`, `minus`, `times`, `div`, `compareTo`, `equals`): `get`, `set`, `iterator`, `contains` and `invoke` are on the agreed list, and leaving them out would leave `List` the only indexable, iterable type, against "library types written in Prism".
- **All of Kotlin's conventions**: the deferred ones need features Prism lacks (custom ranges, delegated properties, hashing) or rules not yet decided.

### D3. Built-in operators first, then the receiver's operator function through the file and its imports (autonomous; the import rule is the user's explicit-library direction applied to operators)

An operator first applies its built-in meaning whenever the built-in operator accepts the operand types after their `&`, `&mut` and `Heap<T>` layers: built-in operators act as members, which win over extensions ("Members Take Precedence Over Extensions"). Only otherwise does it look up the operator function on its **receiver's type**: the left operand of a binary operator, the right operand of `in`, the indexed, called or iterated value. That lookup is exactly a method call's: members (an intrinsic operator member such as `List<T>.contains`), then the file's own extensions, then its imports' (several imports: `E4012`). So an operator function, like any extension, is used only where its module is imported, which the user's explicit-library direction ("explicit imports everywhere") implies; a library type's operators come with the import of its module, which a file using the type's constructors or `dp` imports anyway. When nothing is found the use is `E4002` (`OperatorMismatch`) with help naming the function to declare, and explicit-library's import help (D8 there) extends to it: an operator function a library module declares gets "add `import /prism/...`".

Because the built-in meaning is decided first from the operand types, an operator function the built-in operator would always shadow can never be called. Like an extension shadowed by a field, it is `E4010` at its declaration: arithmetic, unary and ordering functions on `Int`, `Double`, `Char`, `Bool` and `String` whose operand types the built-in accepts, and `get`, `set`, `iterator`, `hasNext`, `next` or `contains` on `List` and `Range` (`get` and `set` on `String`). Functions for other operand types are allowed, so `operator fun Int.times(other: Dp): Dp` serves `2 * 8.dp`, as Compose's does; `1 + 2.0` stays `E4002`.

One function per receiver type constructor, name and context, as for every extension: `Dp.times(Double)` and `Dp.times(Int)` cannot coexist. Compose has both, but overloading by parameter type would need overload resolution at every use (and per argument), which the compile-speed principle rejects; the library picks one (`Dp.times(Double)`, with `Int.dp` for integers), and overloading is a follow-up.

**Compile-time cost.** The built-in test is today's type comparison. When it fails, one hash lookup in the member table and at most two in the extension tables (own, then imports), as a method call; the `E4010` check at a declaration is one comparison against a fixed table. Linear in the number of uses.

Rejected:
- **Operator functions first, built-ins as fallback** (Kotlin's members of `Int` are ordinary operator functions): every arithmetic expression would pay a lookup, and an import could change what `1 + 2` means.
- **Operator functions found everywhere, without import** (as Kotlin members are, since they belong to the class): a prelude of operators for every library type, against explicit-library D1/D3; only `equals`, which must be global to be coherent, is part of its type (D6).
- **Looking up the right operand's type too** (Python's `__radd__`): two lookups and an ambiguity rule; `Int.times(Dp)` covers the case.
- **Overloading by parameter type**: overload resolution per use; deferred.

### D4. An operator use is exactly the call it spells (autonomous)

`a + b` is typed, borrowed and evaluated as `a.plus(b)`: the receiver is passed by the function's receiver mode as a method receiver is (a place borrowed where it lives, anything else a temporary; moved for a `consuming` receiver), the other operand as an argument of its parameter (implicitly borrowed for `&T`, copied or moved otherwise), and the result is tied to the inputs by the elision rules (rule 2: to the receiver only). Operands are evaluated left to right whichever is the receiver (`a in b` evaluates `a` first, as written). A nullable receiver is not looked through: `a + b` for `a: Vector2?` is `E4002`, with help suggesting `a?.plus(b)`, since a nullable result from `+` would surprise. So `Name + Name` with `operator fun Name.plus(other: &Name)` moves nothing, and a library type chooses copy, borrow or move per operand through its signature, with no rule of its own.

**Compile-time cost.** That of a call; nothing beyond the receiver classification calls already do.

Rejected:
- **Operands always borrowed** (operators never move): an owning type could not reuse its receiver's buffer (`operator consuming fun Text.plus`), and the signature would stop saying what happens.
- **Safe operators on nullables** (`a + b` giving `R?`): Kotlin does not either; `?.plus` is explicit.

### D5. Signatures are checked against their convention, with one new code `E4035` (autonomous)

An `operator` declaration must be an extension named by a convention, without `compose`, a context parameter or default values (except `invoke`), with its convention's shape:

| Convention | Receiver modes | Parameters | Result |
|---|---|---|---|
| `plus`, `minus`, `times`, `div`, `rem` | `fun`, `consuming fun` | 1 | any |
| `unaryMinus`, `not` | `fun`, `consuming fun` | 0 | any |
| `compareTo` | `fun` | 1 | `Int` |
| `equals` | `fun` | `other: &T` | `Bool` (and D6) |
| `contains` | `fun` | 1 | `Bool` |
| `get` | `fun`; `mut fun` (D9) | 1 or more | any; `&mut E` for `mut fun` |
| `set` | `mut fun` | 2 or more (last is the value) | `Unit` |
| `invoke` | any | any, defaults allowed | any |
| `iterator` | `fun` | 0 | a cursor type holding no reference |
| `hasNext` | `fun` | `cursor: &K` | `Bool` |
| `next` | `fun`; `mut fun` (D11) | `cursor: &mut K` | any; `&mut E` for `mut fun` |

Anything else is **`E4035` `InvalidOperatorDeclaration`**, the next free code of the type checker's range (`E4030` to `E4034` are taken; the open packets take no new `E40xx` code), with a message naming the convention and the rule broken. A use that cannot be typed (a missing part of the `for` protocol, a cursor type that `hasNext` does not take) is reported at the use with the existing codes (`E4002`, `E4004`, `E4003`), since the declarations may each be valid. Type parameters are allowed (`operator fun <T> Bag<T>.get(index: Int): &T`), except on `equals` (D6). `compose` operators (a composable `invoke`) and context parameters are deferred: they would need the compose transform and the context rules to follow operator syntax, for no library need yet.

**Compile-time cost.** A fixed check per `operator` declaration, against a table of fifteen names.

Rejected:
- **Kotlin's looser shapes** (`compareTo` with any parameter is kept; `equals(other: Any?)` is not possible without `Any`): Prism's `equals` takes `&T` (D6).
- **Reusing `E4004`/`E4010` for bad declarations**: neither names the fix (the convention's shape); `E4035` is one code for one kind of mistake.
- **A parse error for `operator` without a receiver**: every operator rule in one place (the checker) and one code.

### D6. `equals` belongs to its type and replaces structural equality everywhere (autonomous)

`==` is structural today, and equality is used far from any `==`: equality glue compares fields of other types, nullables, boxes and list elements, and compose compares kept arguments and written state values through it. If `equals` were an extension found by scope, `a == b` and `listOf(a).contains(b)` could disagree, and two files could compare the same values differently. So an `equals` operator is part of its type's definition:

- It is declared as `operator fun T.equals(other: &T): Bool` **in the file declaring `T`**, a struct or enum, **non-generic**, **with `T`'s visibility**, at most once (`E4035` otherwise). A built-in type cannot have one (it is declared in no file).
- **Every `==` and `!=` of `T` uses it, in every file, with no import**: directly, as a field or payload of another type, inside nullables and boxes, in list `==`, `contains`, `indexOf` and `in`, in `when` value patterns, in compose's comparisons of kept arguments (as Compose compares parameters with `equals()`) and of state writes. `!=` negates it.
- `==` stays a binary equality node in the typed tree and MIR (D14); the code generator's equality expansion and glue call the function when they meet `T` (D16). Phase 1 records the operator in `T`'s ABI record and reachable definition (D15), so an importer's code for a type holding a `T` calls it.
- Generic types are deferred: the instance of `Box<Int>.equals` would have to be planned from glue that no body names, which the per-program instance planner (`compilation-and-abi`) does not see. A non-generic `equals` is a plain function with one symbol.
- `==` on a type parameter stays `E4002`: this is not a bound; equality as a trait is still future work, and an `equals` operator is what such a trait would call.

An `equals` that compares with `==` of its own type recurses (a runtime stack overflow, as for any recursion); its body compares fields.

**Compile-time cost.** Phase 1: one flag per struct or enum, from a syntactic match of an `operator fun T.equals` declaration in the same file (a hash lookup per such declaration). Code generation: one flag test per type part in the equality expansion, which is already linear in the parts compared; a call replaces an expansion, so code is not larger.

Rejected:
- **`equals` as an ordinary scoped operator extension**, used by `==` only where visible: incoherent across files and between `==` and glue, as above.
- **Using `equals` for direct `==` only, with glue staying structural**: `listOf(a).contains(b)` and `Team(a) == Team(b)` would disagree with `a == b`.
- **Deferring `equals`**: the agreed list names it, and library types whose identity is a field (a handle with a generation, a cached measurement) need it.
- **Allowing it in any file of the type's package**: the type's ABI could no longer be extracted from its file alone (the single-file compilation invariant).

### D7. `compareTo` serves the four ordering operators (autonomous)

`a < b` is `a.compareTo(b) < 0`, and likewise `<=`, `>`, `>=`, with the `Int` result compared with `0` by the built-in comparison (Kotlin's rule). Comparisons stay non-associative. `==` never calls `compareTo`, as in Kotlin, since types such as `Double` order and equate differently.

**Compile-time cost.** One lookup per comparison that is not built in.

Rejected:
- **Separate `lessThan`, `lessThanOrEqual`, ... functions**: four declarations for one ordering, not Kotlin's names.

### D8. `in` and `!in` are binary operators calling `contains` (autonomous)

Kotlin's `x in xs` needs an expression form Prism lacks. `a in b` calls `b.contains(a)` and `a !in b` negates it. Grammar: both bind looser than `?:` and tighter than comparisons, as Kotlin's named checks do, and are non-associative like comparisons (`a in b in c` tests a `Bool` for membership, almost always a mistake; parentheses allow it). The lexer reads `!in` as one token when no identifier character follows `in` (`!inside` stays `!` and a name). `for (x in xs)` and `when` range patterns keep their `in`: the `for` header parses a name before `in`, and a pattern starts with `in`. On a `Range`, membership is built in (`start <= a` and `a` below the end, or at it for `..`), as the `when` range pattern already tests; on a `List`, `contains` is its intrinsic member, which the library now declares `intrinsic operator` (the intrinsic catalog records the flag, and a mismatch is `E4029`). `in` patterns over containers (`when (x) { in xs -> }`) and `!in` patterns are deferred; they are a pattern kind beside `InRange` and have no library need.

**Compile-time cost.** Parsing: one more precedence level. Checking: one lookup per `in` that is not a range.

Rejected:
- **No `in` expression** (`xs.contains(x)` only): the ROADMAP lists `contains` for `in`, and `x in 0..<n` is the idiomatic Kotlin range test.
- **`in` at the precedence of comparisons**: Kotlin places it between `?:` and comparisons; matching it keeps Kotlin code meaning the same.

### D9. Indexing: `get` returns a borrow or a value; a shared and a mutable accessor (autonomous)

Kotlin's `get` returns a value, which in Prism would copy a `Copy` element and could not return a non-`Copy` one at all (no moving out of a collection). So `get` may return a **borrow** `&E`, which makes `a[i]` a **place** of type `E` behind that borrow, exactly as a list element is a place: a `Copy` element is copied out where read, `&a[i]` is the borrow `get` returned, `a[i].name` reads the field in place, and a method call borrows it as its receiver. The borrow is tied to `a` by elision rule 2, so holding `&b[0]` across `b.add(...)` is `E5003` as for a list. A `get` returning a **value** (`Matrix.get(row, column): Double`) makes `a[i]` that value, a temporary.

Writing needs `&mut E`. A type may therefore declare **two `get`s for one receiver, a shared one returning `&E` and a `mut` one returning `&mut E`**, which Rust calls `Index` and `IndexMut`. They are the only extensions besides `next` (D11) that may differ by receiver mode alone; the extension table keeps the `mut` one beside the shared one, as it keeps extensions differing by context. Where a list element would need a mutable list (a field write `b[0].done = true`, `&mut b[0]`, a `mut fun` receiver `b[0].reset()`, a compound assignment of a field, an assignment without `set`), `b[0]` calls the `mut` one; elsewhere the shared one, or the `mut` one when it is the only `get`. A mutable position with no `mut` `get` is `E4003`, with help naming the declaration to add. A plain call `b.get(0)` resolves to the shared one.

The checker learns whether a position is mutable only after typing the place (a method's receiver mode is known once the method is resolved, which needs the element type). It types `b[0]` with the shared accessor's element type and keeps the call's node; the existing paths that demand a writable place (and report `E4003` today) switch that node's callee to the `mut` accessor and its receiver borrow to mutable. When a file declares both, `E4035` requires matching parameter types and element types (`&E`, `&mut E`).

Indices are a list (`m[r, c]`, Kotlin's syntax) passed in order; `List` and `String` take exactly one (`E4004` otherwise).

**Compile-time cost.** At most two lookups per index place and one node rewrite when a writable place is demanded; no pass over the body.

**Implementation (apply).** The `mut` accessor is kept in the extension tables, and named in its declaration path, as `get#mut` (or `next#mut`), beside the shared `get` of the same receiver constructor, name and context: the two would otherwise share a symbol. The dump therefore shows `main::Bag.get#mut`, as it shows `#context(...)` for extensions differing by context. The ABI keeps the written name with the receiver mode, and sorts symbols by mode after name, receiver and context, so the pair has a fixed order. A plain call `b.get(0)` falls back to the `mut` one when it is the only `get`. The declaration checks of D5 live in `crates/prism-typechecker/src/checker/operators.rs`, a submodule of the checker so they reach its tables, rather than `src/operators.rs`.

Rejected:
- **Kotlin's value-returning `get` only, writes through `set` only**: no `x[i].field = v` without copying the element out and back (impossible for non-`Copy` elements), no `&x[i]`; the user asked for places.
- **A `get` returning `&mut E` only**: every read would need a mutable receiver, so a shared `&Bag` could not be read by index.
- **A separately named mutable accessor** (`getMut`): not Kotlin's surface, and every type would need users to know two names; the mode pair keeps `x[i]` uniform.
- **Choosing the accessor by a backward pass over uses**: a pass per body; the demand-time switch is local.

### D10. `set` serves whole-element writes when declared (autonomous)

`a[i] = v` calls `a.set(i, v)` when the type declares `set` (a `mut fun` whose last parameter takes the value), and otherwise writes through the `mut` `get`, dropping the old element and moving the new one in, as for a list element. `a[i] op= v` calls `set` with `get`'s element (copied out of a borrow) combined with `v` by `op`, the index arguments evaluated once into temporaries (copied when `Copy`, borrowed when the parameter is a reference, `E4017` otherwise), or, with no `set`, updates in place through the `mut` `get`. `set` wins when both exist because a type that declares it wants to see every whole-element write (a state-tracking list that invalidates readers, for example); field writes and borrows still go through the `mut` `get`, since `set` cannot express them.

**Compile-time cost.** One lookup per indexed assignment.

**Implementation (apply, autonomous).** `a[i] op= v` through `set` is a block: each index into a local (`index#1`, a name no source can write), then `a.set(index#1, a[index#1] op v)`, the inner `a[index#1]` an ordinary index through `get`. The indexed value is read by both calls, so it must be a variable or a field path, which reads the same value twice (`E4003` otherwise, with help to hold it in a `var`); an index that both functions take by value must be `Copy` (`E4017`). Rejected: holding any indexed expression in a temporary, for a case (`make()[0] += 1`) whose `set` would write a value dropped at once.

Rejected:
- **The `mut` `get` first, `set` as a fallback**: a type could not observe writes it declared `set` for.
- **Kotlin's rule that a compound assignment through `set` re-evaluates the indices**: Kotlin evaluates them once too; Prism states it.

### D11. `for` uses a cursor protocol on the collection (autonomous)

Kotlin's `for` calls `iterator()` and then `hasNext()` and `next()` on the iterator object. In Prism an iterator object borrowing its collection would be a struct holding a borrow, which structs cannot (and lifting that needs lifetime annotations, against the zero-cost memory invariant). So the collection itself serves the protocol, with a separate **cursor** value as the iteration state:

- `operator fun C.iterator(): K` returns the initial cursor, a value of a type holding no reference (an index, a node handle, a small struct);
- `operator fun C.hasNext(cursor: &K): Bool` tests it;
- `operator fun C.next(cursor: &mut K): E` returns the element and advances the cursor; `E` is typically `&T`, tied to the collection by elision rule 2, or a value.

`for (x in y)` evaluates `y` once and borrows it for the whole loop (a place where it lives, a temporary kept to the loop's end), as a list loop does, so changing `y` in the body is `E5003`; then `var k = y.iterator(); while (y.hasNext(&k)) { val x = y.next(&mut k); body }`, dropping `k` at the exit. `x` is a fresh binding of `next`'s result type: a borrow like a list loop's `&T`, or an owned value dropped at the end of its iteration. `for (x in &mut y)` calls a second, `mut` `next` returning `&mut E` (the mode pair of D9), with `iterator` and `hasNext` called through shared reborrows; its `x` is dead before the next call, so the reborrows never overlap. Nothing allocates unless the cursor type owns storage, and the calls are direct calls through the function table.

**Compile-time cost.** Three lookups per loop; MIR adds one loop shape (D14).

**Implementation (apply, autonomous).** A cursor loop iterates mutably exactly when a list loop would: when the iterable has a `&mut` type, written `&mut b` or a `b: &mut Bag` parameter (`collections` D10). The proposal's scenario wrote `for (item in &mut b)` for a `b: &mut Bag` parameter, which is `E4003` (a `&mut` of a `val` binding); the scenario now shows both forms, and a shared loop variable's field write gets the help `for (item in &mut b)`, for list loops too. Unlike a list, a cursor type may have no `mut` `next`: a value of a `&mut` type then iterates shared, and only a written `&mut b` without one is `E4003`. The loop's header evaluates `hasNext` in its own scope, so its borrows end before the branch.

Rejected:
- **Kotlin's iterator objects**: need a struct holding a borrow.
- **An index protocol** (`size` plus `get(i)`, like a list loop): covers random-access types only; it is the cursor protocol with an `Int` cursor, which `Bag` in the scenarios uses.
- **A closure protocol** (`forEach { }` as the loop): `break`, `continue` and `return` would not work, and a closure call per element.
- **Generators** (`yield`): a coroutine transform, far beyond this change.

### D12. `invoke` serves calls of values of non-function types (autonomous)

A call whose callee is a value (local, parameter, field, top-level `val` or any expression) whose type after its layers is not a function type calls that type's `invoke` with the call's arguments, named arguments, defaults and a trailing lambda included. Any receiver mode is allowed: `operator mut fun Counter.invoke()` makes `c()` need a mutable `c`, by the method-receiver rules. Name resolution is unchanged: a local or field still shadows a function of its name, and only then is its type's `invoke` consulted. A field call `b.onClick()` calls the field (members first), and `invoke` when the field's type is not a function type.

**Compile-time cost.** One lookup per call whose callee value is not a function, which is an error today.

Rejected:
- **Leaving `invoke` out**: it is on the agreed list, and costs nothing for other calls.

### D13. Compound assignment through operator functions is `a = a op b` (autonomous)

`a op= b` where the built-in operator does not apply calls the operator function with the target place as its receiver (borrowed, or moved for a `consuming` receiver) and assigns the result back to the same place, evaluated once; the result must have the target's type (`E4002`). Kotlin first looks for `plusAssign`, which is deferred (D2); without it, `a += b` on an owning type with a `consuming` `plus` already reuses the receiver's storage, moved in and assigned back.

**Compile-time cost.** One lookup per compound assignment that is not built in.

**Implementation (apply, autonomous).** A consuming function moves its target out before the result is assigned back, and Prism has no partial moves and no moves out of a borrow or a list: the borrow checker's move paths stop at whole bindings, and a move out of `(*xs)[i]` would leave a hole a shared borrow passed as the value could still see (`xs[i] += &xs[i]`). So a consuming receiver needs a whole binding as the target (`var a = ...; a += t`), and `xs[i] += t` with a consuming `plus` is `E4017`, naming the consuming function; with a shared receiver it borrows the element, and the old element is dropped before the result is assigned. Rejected: taking the element out and putting the result back, which Prism's non-unwinding runtime errors would make sound in itself, but which needs a "take" operand that the borrow checker treats as an exclusive access without a move path, in every MIR consumer; it is a follow-up (`ROADMAP.md`), worth it once owning library types (`drop-and-clone-hooks`) use consuming operators on list elements. A compound assignment through an operator function whose target has a built-in type (`s *= 3` with `operator fun String.times(count: Int): String`) calls the function too, its value inferred once for the built-in test and passed on.

### D14. Typed tree and MIR: marked calls, accessor places and one loop shape (autonomous)

Operator uses become `Call` nodes of the operator function, normalized as extension calls (receiver first, borrowed or moved by mode), with a new mark recording the operator spelled, which `dump-typed-tree` prints (`call operator + main::Vector2.plus`); an ordering use is the built-in comparison of the call with `0`, `!in` a `not` of the call. `==` stays an equality node (D6). An index through `get` returning a borrow is `Deref` of a temporary holding the call (a place, so fields, copies and borrows work unchanged); a borrow of the element, whole or a part, reborrows it (**apply, autonomous**: the proposal made a whole-element borrow the call itself, to spare the borrow checker a reborrow through a temporary; the borrow checker already carries a reference's borrows through such reborrows, as for `&f(x).name` with `f` returning `&Item`, so `fun first(b: &Bag): &Item = &b[0]` checks, and one shape for whole and partial borrows is simpler; rejected: the special case, which needed a node to disappear from the tree). A `set` write is a call statement; an indexed compound assignment through `set` is a block of index temporaries and the call. A compound assignment through an operator function records the function. A `for` over a cursor is `ForIteration::Cursor { iterator, has_next, next, cursor, element, mutable }` with the iterable held as a borrow whose origin is the loop. MIR lowers cursor loops to a header calling `hasNext` and a body calling `next`; everything else is existing calls, temporaries and places, so the borrow checker and the backend need nothing new. The formatter is unaffected beyond printing the new syntax (D17).

**Compile-time cost.** Linear: one node per use as before; the cursor loop lowers in constant time.

Rejected:
- **A dedicated operator node in MIR**: every consumer (borrow checker, compose transform, backend) would learn it, for what is a call.
- **Desugaring in the parser**: the parser cannot know whether `+` is built in, and diagnostics would show calls the user did not write.

### D15. The ABI records operator flags and equality operators (autonomous)

`ExportedSymbol::Function` gains `is_operator`, and `ExportedSymbol::{Struct, Enum}` and `AbiTypeDefinition` gain `equality: bool`, all covered by the ABI hash and the structural hash of definitions (so an instance key whose type arguments reach `User` changes when `User` gains `equals`). Marking a function `operator` changes what importers' syntax reaches, so it must re-check them; editing an operator function's body changes nothing downstream (deterministic hermetic builds). Type fingerprints (hot reload) stay representation-only: a reload that adds `equals` changes the ABI, so dependents recompile, and the glue they call through the function table is the new code.

**Compile-time cost.** A flag per function and per type in Phase 1.

### D16. Equality expansion and glue call `equals` through the function table (autonomous)

In `crates/prism-codegen/src/equality.rs`, expanding equality of a struct or enum whose definition has an equality operator emits a call of that function (by its symbol's slot, as any call), passing the two values' addresses (the parameter is `&T`, the receiver a shared borrow), counting one comparison of the inline budget; glue functions (`<module>#equals(<type>)`, index-of glue, kept-value and state-record equality entries) reach it through the same expansion. A type with `equals` gets no structural glue of its own.

**Compile-time cost.** One flag test per part; code shrinks where a call replaces an expansion.

### D17. Formatting prints the new syntax only (autonomous)

`prism fmt` prints `operator` in the canonical order (visibility, `external`/`intrinsic`, `operator`, `compose`, receiver mode, as the parser requires), index lists like argument lists (`m[1, 2]`, broken as arguments when too long), and `in`/`!in` with a space on each side. The verifier compares the new AST fields. No layout rule changes.

**Compile-time cost.** None for compilation.

### D18. Compile timing (autonomous)

`crates/prism-cli/tests/compile_timing.rs` gains the shape `operator-uses`: files of 256, 1,024 and 4,096 functions, each using `+`, `<`, `in`, `==` on a type with `equals`, a borrowing index place, an indexed compound assignment through `set` and a cursor loop, on user types declared once, asserting growth of at most ×4.6 per ×4 under `PRISM_ASSERT_TIMING=1`, as every stage's shapes do.

**Measured (apply, release build, M-series Mac, the full `PRISM_ASSERT_TIMING=1` run passing).** `operator-uses` at 256, 1,024 and 4,096 functions, in ms: `type` 9.7, 38.5 and 165.8 (×4.0, ×4.3); `mir` 4.7, 20.9 and 93.9 (×4.4, ×4.5); `borrow` 24.7, 100.0 and 425.6 (×4.0, ×4.3); `lower` 10.6, 42.8 and 175.6; `clif` 21.1, 83.9 and 335.2; `compose` 1.1, 5.5 and 29.9 (×4.8, ×5.4, the small stage's memory effects that `list-loops` shows too, ×5.3). Every stage stays within the assertions; the type checker's operator lookups cost about 40 µs per function of seven operator uses.

## Risks / Trade-offs

- **[A reborrow through the accessor's temporary]** `&b[0].name` derives from a temporary holding `get`'s `&Item`, whose storage ends with the statement. The borrow checker must treat the dereference as reaching `b` (references carry their borrows), not the temporary, or `fun first(b: &Bag): &String = &b[0].name` would be a false `E5006`/`E5008`. → D14 makes a whole-element borrow the call itself; field paths rely on "References Carry Their Borrows"; scenarios cover both, and the reviewer should probe them.
- **[`equals` runs user code inside glue and the compose runtime]** A panicking or slow `equals` now runs from list searches, state writes and kept-argument comparisons. → As in Compose, which calls `equals()` there; runtime errors are located as any; documented in the `operators` rationale.
- **[Two `get`s with different element types]** Imported shared and mutable accessors from different files could disagree. → The element type comes from the shared accessor; a mutable position whose `mut` accessor gives another type is `E4002`; same-file pairs are checked by `E4035`.
- **[One function per name]** `Dp.times(Int)` and `Dp.times(Double)` cannot coexist. → Recorded deviation from Compose for `library-in-prism` to document in the `ui` spec; overloading is a follow-up.
- **[`in` as an operator]** `in` is already a keyword, so no identifier is lost; an expression statement starting a line with `in` was a syntax error before and still is (a newline before `in` ends the expression).

## Verification (apply)

The flagged risks, probed with small programs under `--check-allocations`: reborrows through an accessor's temporary check and run (`fun first(b: &Bag): &String = &b[0].name`, `fun whole(b: &Bag): &Item = &b[0]`, a `&mut b[0]` written through), with no false `E5006` or `E5008`; holding `&b[0]` across `b.add(...)` is `E5003`. A user `equals` runs inside equality glue and the compose runtime: it serves list `==`, `contains` and `indexOf`, boxes, nullables, fields, enum payloads, `when` value patterns, a state write's equality check (an equal write invalidates nothing) and a kept argument's comparison (an equal argument skips the composable); an `equals` that reads a state works outside and during a pass, and one that writes a state works outside a pass and is the located runtime error `state written during composition` during one, never a runtime panic. Generic accessors, a generic cursor protocol and a generic `plus` with its compound assignment run. `a in b` evaluates `a` before a temporary `b` (MIR evaluates a receiver ordered after an argument in its turn). The apply checks of `openspec/config.yaml` pass: `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `prism check` and `prism fmt --check` over `examples/`, `examples/run/main.prism` and the four UI entries headless under `--check-allocations`, and the full `PRISM_ASSERT_TIMING=1` compile-timing run.

## Migration Plan

No migration: every program accepted today keeps its meaning, since operator functions apply only where built-in operators do not and `operator` is a new modifier. The intrinsic `List<T>.contains` gains `operator` in `library/prism/collections.prism`. Rollback: revert the commits.

## Open Questions

- Should overloading by parameter type come for operators only (bounded: the right operand's type constructor as a second key), so the library can follow Compose's `Dp.times(Int)` and `Dp.times(Float)`? Decide in `library-in-prism` if the single form proves awkward.
- When traits arrive, `equals` and `compareTo` become conformances; the conventions are designed to be what those traits call.
