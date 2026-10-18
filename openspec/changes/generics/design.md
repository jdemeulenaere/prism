# Design: generics

## Context

See `proposal.md` for motivation. This is item 10 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17. It also covers the Phase 2 item "Generics: type parameter checking and monomorphization" (deferred by `type-checker-core`), minus monomorphization (D1). The user was not available while this change was proposed. Every decision beyond what existing specs and invariants imply is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

What exists today:

- **Parser** (`crates/prism-parser/src/parser.rs`). `parse_optional_type_params` reads `<T, U>` as bare names after a function's name (`fun id<T>`, and after an extension's name, `fun User.f<T>`), after a struct's or enum's name, and accepts `<>`. Bounds are not parsed. A call cannot carry type arguments. A receiver with type arguments (`fun Heap<User>.size()`) is `E2001`. Comparisons are left-associative (`Precedence::Comparison`), so `a < b < c` parses and then fails to typecheck. The lexer has no `>>` token, and `expect_closing_angle` splits a joined `>=`.
- **AST** (`crates/prism-ast`). `type_params: Vec<String>` on `FunctionDecl`, `StructDecl` and `EnumDecl`. `TypeKind::Named { name, type_args }`. `ExprKind::Call { callee, args }`.
- **ABI** (`crates/prism-abi/src/signature.rs`, `abi_type.rs`). `type_params: Vec<String>` on each symbol, hashed by name. Inside a signature a type parameter is lowered like any name, so it is `AbiType::Unbound`, or binds to a struct of the same name. `AbiType::Nominal` already has `args`. An exported function with type parameters records `generic_body_hash`, a span-free hash of its body's AST (the "Monomorphization Hash" requirement of `compilation-and-abi`).
- **Types** (`crates/prism-typed-tree/src/types.rs`). `Type::TypeParam(String)` exists, but only the `Copy`/`Clone` tables and a CLI help mention it. `Type::Struct(NominalId)` and `Type::Enum(NominalId)` have no arguments. `type_from_ast` (`crates/prism-typechecker/src/types.rs`) keeps arguments only for `Heap<T>` with exactly one argument. Every other named type becomes `Type::Opaque(name)` and loses its arguments. So inside `fun id<T>(x: T): T`, `T` is an unknown opaque name, `Box<Int>` and `Box<String>` are the same type, and `Heap<A, B>` is silently an opaque `Heap`.
- **Checker** (`crates/prism-typechecker/src/checker.rs`). Calls check each argument against its parameter type: `check_call_arguments` calls `check_argument`, which for reference parameters infers the argument first and then `fit`s it, deferring the move check. Constructions go through `construct_struct`. Extensions are keyed by `(receiver Type, name)` in `own_extensions` and `imported_extensions`. Markers are checked by `check_marker_declarations`, which today rejects `struct Box<T> : Copy { val v: T }` (`E4016`) because `T` is opaque.
- **Typed tree and MIR.** `Call.parameters` records each parameter's name and type (D18 of `typed-tree`), and `TerminatorKind::Call.parameters` its type. `needs_drop` (`crates/prism-mir/src/types.rs`) is "not `Copy`". Elision reads the parameter types (`crates/prism-borrowck/src/elision.rs`), and the escape check (`escapes.rs`) inspects only statements (returns, aggregates, projected writes, captures).

## Goals / Non-Goals

**Goals:**
- A generic declaration is checked once, where it is declared, against its bounds alone, and every use is checked against its signature alone. No error ever comes from an instantiation, unlike C++ templates.
- Type parameters flow unchanged through the typed tree, MIR, borrow checking and drop elaboration, so `monomorphization` only substitutes and resolves type-parameter drops, and never rechecks.
- What an importer needs (type parameters, bounds, which parameters condition a marker) is in the ABI, read from the AST alone, which keeps the single-file compilation units and deterministic hermetic builds invariants.
- Syntax a Kotlin user writes without thinking (`fun <T>`, `mutableStateOf<String?>(null)`), parsed by one deterministic rule.

**Non-Goals:**
- Generating code for instantiations, shipping generic bodies to importers, and deciding what the Monomorphization Hash must cover across files (D15, `monomorphization`).
- Traits, trait bounds, `where` clauses, associated types, variance annotations, higher-kinded types, rank-N function types, specialization.
- Rejecting unknown type names (`E4001`). This change provides the generic scoping that item waits for, but it stays separate because it touches every opaque-name fixture.

## Decisions

### D1. Monomorphization is its own change after `runtime-strings-and-drops` (autonomous)

This change goes up to elaborated, polymorphic MIR and the ABI's generic signatures. Instantiating needs four things that do not exist yet: a backend that consumes the instances, drop glue for the type-parameter drops it resolves, the artifact cache, and a way to ship the generic bodies themselves. So `monomorphization` is added to `ROADMAP.md` after `runtime-strings-and-drops` (item 16), and `codegen-core` and `codegen-aggregates` lower only non-generic bodies until then. Rejected: delivering an instantiation collector and MIR substitution now, tested through `dump-mir`. Where instances live is an open design question (D15), and the collector's shape depends on its answer, so it would likely be rewritten. Rejected too: folding monomorphization into `codegen-core`, which is already the largest backend step.

### D2. Kotlin declaration syntax: `fun <T> name` (autonomous)

Type parameters go between `fun` and the name, or the receiver: `fun <T> id(x: T): T`, `fun <T> Box<T>.get(): &T`, `compose fun <T> Row(items: &Box<T>)`. Structs and enums keep `struct Box<T>` and `enum Option<T>`. A list is one or more names, each optionally bounded, with an optional trailing comma. `<>` is `E2001`. The old placements, `fun id<T>()` and `fun User.f<T>()`, are `E2001` with a message showing `fun <T> id()`.

The type parameters have to come before the receiver because a generic receiver mentions them (`Box<T>`). Kotlin chose this form for that reason, and it is the form Kotlin users know. Rejected: keeping `fun id<T>` (Rust and Swift) and adding `fun <T>` only for extensions, which gives two placements, and the receiver form would still read `T` before declaring it.

### D3. Explicit type arguments: Kotlin's rule, made deterministic (autonomous)

At a call, `f<Int>(x)`, `Box<String?>(null)`, `remember<Int> { 0 }` and `s.map<Int> { n -> n + 1 }` give type arguments to the function, constructor or extension being called. In expression position, after an identifier or after a `.`/`?.` member name, a `<` is tried as a type-argument list. The parser commits to it when the tokens up to the matching `>` parse as types separated by commas, the list is closed by `>` (not by `>=`), and the next token is a `(` or `{` on the same line. Both are the tokens that already start a call. Otherwise the parser rewinds, and the `<` is the comparison operator. The speculative parse never splits a joined `>=` and reports nothing, and any error in it rewinds. A committed list is attached to the call (`ExprKind::Call::type_arguments`).

Comparisons become non-associative, as in Kotlin's grammar: `a < b < c` and `a < b > c` are `E2001`, with a message that suggests `&&`. They never typechecked, since `Bool` is not ordered. With that rule, `f < A > (x)` has no comparison reading left. The only remaining double reading is a pair of comparisons split by a comma inside an argument list, `g(a < b, c > (d))`. Kotlin and C# resolve it as a generic call, and so does Prism, as a fixed rule with a scenario. Writing `g((a < b), c > (d))` gives the comparisons. This is a disambiguation rule like `else` binding to the nearest `if`: deterministic and documented, so the Kotlin-like syntax without ambiguity invariant holds.

Rejected:
- Rust's turbofish `f::<Int>(x)`. It is fully unambiguous, but foreign to Kotlin users, who write `mutableStateOf<String?>(null)` and `mutableListOf<String>()` all the time, and it takes `::`, which method references (`u::greet`, a `ROADMAP.md` item) will want.
- Requiring no space before `<`, which would misparse `i<n`.
- Name-directed parsing, where the parser knows which names are generic. It breaks the single pass and the parser's independence from the type checker.

### D4. Types carry parameters and arguments; nullables flatten (autonomous for flattening)

In `prism-typed-tree`:

```rust
pub struct TypeParameter { pub index: u32, pub name: String, pub bound: Option<TypeMarker> }
pub enum Type { ..., Struct { id: NominalId, arguments: Vec<Type> }, Enum { id: NominalId, arguments: Vec<Type> },
                Parameter(TypeParameter), ... }   // TypeParam(String) is removed
pub enum TypeDef { Struct { parameters: Vec<TypeParameter>, fields: ... }, Enum { parameters: Vec<TypeParameter>, variants: ... } }
impl Type { pub fn substitute(&self, arguments: &[Type]) -> Type }
```

A parameter carries its bound, so `TypeTable::is_copy`, MIR and the borrow checker can answer "is this `Copy`?" without a context. Parameters are indexed within their declaration. A type only ever mixes the parameters of one declaration: a struct's fields mention the struct's parameters, and a function's types mention the function's own. `substitute` replaces `Parameter(i)` by `arguments[i]` and normalizes through `Type::nullable`, so `T?` with `T = String?` is `String?`. This is Kotlin's meaning, and it matches the existing rule that `T??` is `T?`. `monomorphization` then rewrites a substituted body where it wraps or unwraps a `T` that turned out nullable (D15). Rejected: Swift's nested optionals (`Optional<Optional<T>>`). They are more exact, but they would make `T??` a new type everywhere and surprise Kotlin users writing `State<String?>`.

### D5. Bounds: `Copy` and `Clone` only, on function type parameters; unbounded means move-only (autonomous)

`<T : Copy>` or `<T : Clone>`, exactly one marker name after `:`, as in a marker declaration. Anything else is `E2001` saying that only `Copy` and `Clone` can bound a type parameter until traits exist. Bounds on struct and enum type parameters are `E2001` ("not supported yet"). They would need a well-formedness check at every annotation, and conditional markers (D6) cover the use case Milestone 2 needs.

In a generic body, a value of type `T` may be moved, borrowed (`&x`, `&mut x`), dropped, passed on, returned, wrapped in `T?` or `Heap<T>`, stored in a generic struct, compared to `null` through `T?`, and given to `?:` and `?.` through `T?`. `T : Copy` is copied wherever a `Copy` value is, and `T : Clone` (or `Copy`) has the built-in `clone()`. Anything else is rejected by the existing codes:

- `x.clone()` on an unbounded `T` is `E4018`, with help suggesting `<T : Clone>`.
- A field or method on `T` is `E4006`.
- `==`, `!=`, `<` or arithmetic on `T` is `E4002`, the operator mismatch code, with help saying that type parameters have no equality until traits exist.
- Moving a `T` out of a field or borrow is `E4017` as for any non-`Copy` type.

`==` is the one decision someone could argue the other way, since every concrete type supports `==` today and a monomorphized `==` would work. It is rejected because equality is bound to become a trait (structural equality of floats and function values is not settled), and adding a bound later would break every generic body that relied on `==`.

### D6. Markers on generic types are conditional on the type parameters their fields use (autonomous)

For `struct Pair<A, B> : Copy { val a: A, val b: B }`, the marker's parameters are the type parameters that occur syntactically in a field or payload type, here `A` and `B`. They are computed from the AST alone (Phase 1a). An instantiation `Pair<X, Y>` is `Copy` when the marker is `Copy` and every marker parameter's argument is `Copy`. It is `Clone` when the marker is `Copy` or `Clone` and every marker parameter's argument is `Clone`. The declaration check (`E4016`) treats each marker parameter as satisfying the marker. So `struct Box<T> : Copy { val v: T }` is accepted, `struct W<T> : Copy { val h: Heap<T> }` is still `E4016`, and `struct Handle<T> : Copy { val id: Int }` is `Copy` for every `T`, which is what `State<T>` needs.

Rejected:
- Unconditional markers that require bounds (`struct Box<T : Copy> : Copy`). Then a `Box<String>` cannot exist, and struct bounds are out of scope (D5).
- Rust's derive rule, which conditions on every parameter, so a phantom handle `State<String>` would not be `Copy`.
- A semantic condition computed from the field types' own markers. It is more exact (a field `Handle<T>` would not condition on `T`), but it needs the imported types' marker parameters in Phase 1a, and the syntactic rule only ever errs towards "not `Copy`".

### D7. Type parameters range over borrow-free types (autonomous)

The soundness question is a generic body checked once against a `T` that is later instantiated with a borrow. Take `fun <T> Team.choose(x: T): T = x`. With `T = Int`, rule 2 ties the result to the receiver, and that is fine. With `T = &Int`, the result derives from `x`, which rule 2 forbids (`E5009`). The body is the same, but the verdict depends on the instantiation. A generic struct cannot hold a borrow either (struct fields cannot, per the parser rule), and the borrow checker could not tell that a `T` value carries loans. Two rules make every value of a type-parameter type carry no loans:

1. **A type argument, written or inferred, must not contain a reference outside a function type**: `E4022` `BorrowInTypeArgument`, at the explicit type argument, the annotation (`val b: Box<&Int>`, `fun f(h: Heap<&Int>)`) or the call whose inference produced it. A generic function that works on borrows says so in its signature (`fun <T> first(a: &T, b: &T): &T`). Inference never proposes a reference for a by-value `T` (D8), so `id(&a)` instantiates `T = Int` and copies out, as passing `&a` to an `Int` parameter does, and `id(&s)` for a `String` is the familiar `E4017` "cannot move out of a borrow". `E4022` therefore comes from explicit arguments, annotations and nested positions such as `apply(f)` with `f: (&Int) -> Unit` for `(T) -> Unit`. `Heap<T>` follows the same rule, since a `Heap` is an owned allocation like a struct.
2. **A function value that may hold a closure's captures or a function input cannot be passed where the callee's declared parameter type has a type parameter outside a function type**: `E5007`, labelled "passed to a generic parameter here", because the generic body may store, return or keep that value. For `fun <T> id(x: T): T`, `id({ s.length })` is rejected and `id({ 1 })` and `id(f)` for a top-level function `f` are accepted. A closure passed for `calc: () -> T` (`remember { count }`) is a function-typed parameter of the callee, so it is not a generic slot and stays allowed. Owning, move-capturing closures (`closures`) will hold no loans and pass.

With both rules a type-parameter value carries no loans, so `places::carries_loans(Parameter)` is false, the elision rules computed on the declared signature give the same answer as on the substituted one, and no generic body needs rechecking. This mirrors Swift, where generic parameters are `Escapable` by default and a non-escaping closure cannot be a generic argument. Relaxing it later (an opt-in bound for borrowed instantiations) only accepts more programs. Rejected:
- Elision applied after substitution, with the generic body checked per instantiation. That is what C++ templates do, and it makes errors depend on callers.
- Checking the body with `T` treated as possibly borrowed. That rejects `Team.choose` for `T = Int` too.
- Forbidding function types as type arguments. `List<() -> Unit>` of non-capturing or owning closures is legitimate UI code.

### D8. Local, bidirectional type argument inference (autonomous)

For a call of a generic function, extension, struct constructor or payload variant, or the use of a generic enum's payload-free variant:

1. **Explicit arguments win.** The count must match (`E4020`), and nothing is inferred.
2. Otherwise each type parameter gets an unknown. **The receiver** of an extension call, at the looked-through layer whose type constructor matches (D10), is matched against the declared receiver type.
3. **The expected type**, when the call is checked against one (not the error type), is matched against the declared return type. This gives exact solutions: `val s: State<Int?> = mutableStateOf(null)` solves `T = Int?`.
4. **Written arguments** are visited in source order. An argument is synthesized when its parameter's declared type mentions an unsolved unknown and it is not a closure literal whose declared function type has a parameter mentioning one. Synthesis is the `check_argument` path for reference parameters: the argument is inferred, and its move check is deferred. Its type is then matched against the declared parameter type, giving lower bounds. Matching looks through an implicit borrow (declared `&T`, found `Int` gives `T >= Int`), strips references from a found type matched against a by-value unknown (declared `T`, found `&Int` gives `T >= Int`, D7), matches `T?` against `U?` or `U`, ignores `Nothing` and `Nothing?` at a nullable position, and descends into nominal arguments and function types. Other arguments are checked once the unknowns they mention are solved.
5. **Solve.** An unknown with an exact solution takes it, and otherwise the join of its lower bounds (the existing `join`, so `pick(1, null)` gives `Int?`). Lower bounds that have no join are reported at the argument that breaks the join, as `E4004`.
6. **Deferred closures** are checked against their declared function type with the solved unknowns substituted. If a parameter type still mentions an unsolved unknown, that is `E4013`. If the return type does, the body is inferred and binds it (`remember { 0 }` gives `T = Int`, `map(1) { n -> n + 1 }` gives `R = Int`).
7. Any unknown still unsolved is **`E4013` `CannotInferType`**, naming the type parameter and the callee, with help to write `name<...>(...)` or annotate the expected type (`val b = Option.None`, `fun <T> make(): Box<T>` called as `make()`).
8. The solution is checked for **borrow-freedom (`E4022`) and bounds (`E4021`)**. Then every synthesized argument is fitted to its substituted parameter type with `fit`, which reports mismatches as `E4004` and runs the deferred move checks.

A generic function used as a value (`val f = id`) is `E4013`, with help to wrap it in a closure. Rejected: Kotlin-style global constraint solving across statements, and Hindley-Milner, which would make inference depend on later uses of a value. The local algorithm needs no inference variables in the typed tree, and its errors point at the call.

### D9. One polymorphic typed tree and MIR per declaration; calls record declared types and type arguments (autonomous)

- `FunctionItem`, `StructItem` and `EnumItem` gain `type_parameters: Vec<TypeParameter>`. A MIR `Body` gains `type_parameters`: the owner function's, also for its default generators and closures, and none for a top-level `val`. The validators check that every `Type::Parameter` in a body or item belongs to its declaration, with the same name and bound.
- A typed-tree `Call` and a MIR `TerminatorKind::Call` gain `type_arguments: Vec<Type>`, empty for a non-generic callee. They may mention the caller's own parameters inside a generic body. `parameters` now holds the **callee's declared** parameter types, and the slot type of argument `i` is `parameters[i].substitute(&type_arguments)` (`Call::parameter_type(i)`). A typed-tree `ConstructField.ty` is likewise the declared field type, and the construction's own type (`Box<Int>`) gives the arguments. Recording the declared types keeps calls self-describing (D18 of `typed-tree`), lets `monomorphization` substitute only the caller's `type_arguments`, and lets the borrow checker see which slots are generic (D7, rule 2) and compute elision on the declared signature. Rejected: recording only the substituted types, which hides the generic slots. Also rejected: recording both lists, which would need a rule to keep them consistent.
- Types in bodies are polymorphic. A drop of a value whose type is or contains a parameter that is not `Copy` stays in the MIR as lowered and is elaborated as usual. It "maybe needs drop", and `monomorphization` resolves it to the instance's drop glue or removes it. `needs_drop(Parameter)` is `!is_copy`, so a `T : Copy` value is never dropped.
- The dumps print type parameters as declared (`fun <T : Copy> main::twice`), type arguments after the callee's path (`call main::id<Int>(move _1)`), and nominal arguments in types (`Box<Int>`). The MIR body header shows the body's type parameters.

### D10. Generic extension receivers (autonomous)

A receiver is a type name with optional type arguments, which may mention the function's type parameters (`fun <T> Box<T>.get(): &T`) or be concrete (`fun Box<Int>.sum(): Int`). Extensions stay keyed by the receiver's type constructor (the nominal identity or built-in type, without arguments) and name. So `fun Box<Int>.sum()` and `fun <T> Box<T>.sum()` in one file are `E4010`, `DeclarationPath::receiver` stays unique per file, and an import ambiguity is still `E4012` per constructor. The path's receiver is the declared receiver type (`lib::Box<T>.get`).

A call looks through `&`, `&mut` and `Heap<T>` layers as today. At the first layer whose constructor has an extension of that name, it matches the declared receiver against the layer's type (D8, step 2). A failed match (`Box<Int>.sum` on a `Box<String>`) is `E4006`, with help naming the declared receiver. Explicit type arguments of an extension call list all of the extension's type parameters in order. A bare type-parameter receiver (`fun <T> T.also()`) and a `Heap<...>` receiver stay `E2001`: the first would make every extension lookup generic, and the second competes with looking through `Heap` layers. Rejected: Rust-style `impl<T> Box<T>` blocks, which do not fit extension-only methods.

### D11. User generic types are invariant; `Heap<T>` stays a built-in constructor (autonomous)

`Box<Int>` is assignable to `Box<Int?>` only if `Int` equals `Int?`, which it does not. Generic structs and enums are invariant in every parameter, because a value type's layout depends on its arguments: covariance would hide a representation change behind assignability, the problem `typed-tree` D6 recorded for `Heap<Int>` versus `Heap<Int?>`. `Heap<T>` keeps its own variant (`Type::Heap`), its auto-deref, its `Deref` projection and its covariance, which the `Heap<T>` item in `ROADMAP.md` still has to decide. It joins the arity check (`E4020`) and the borrow-free rule (D7). Rejected: making `Heap<T>` an ordinary generic built-in struct. That removes none of its special cases and would change every MIR `Deref` of a box.

### D12. Type parameter scopes and names (autonomous)

A function's type parameters are in scope in its receiver, parameters, defaults, return type and body (closures included). A struct's or enum's are in scope in its fields or payloads. A type name resolves first to a type parameter of the enclosing declaration, then to the file's and imports' types as today, so a parameter may shadow a struct of the same name. A type parameter named like a built-in type the lowering fixes (`Int`, `Float`, `Double`, `Bool`, `Char`, `String`, `Unit`, `Nothing`, `Heap`) and a duplicate (`<T, T>`) are `E4010`. `Range` stays shadowable, as it is for structs. A type parameter with arguments (`T<Int>`) is `E4020`. Unknown names stay opaque, as today.

### D13. ABI: structured type parameters, positional references, marker parameters (autonomous)

- `ExportedSymbol::{Function, Struct, Enum}::type_parameters: Vec<AbiTypeParameter { name, bound }>`.
- `AbiType::Parameter { index, name }`. `AbiType::from_ast` takes the declaration's type parameter names and lowers a name that matches one, with no arguments, before binding to the file's or imports' types. So `T` never binds to a `struct T`.
- `NominalHeader` and `AbiType::Nominal` gain `marker_parameters: Vec<u32>` (D6), computed by one function over the AST, `prism_abi::marker_parameters(decl)`, which the type checker also uses for the file's own declarations, so the two cannot drift.
- The hash covers the number of type parameters and each bound, `Parameter` by index, and marker parameters. Type parameter names are recorded for diagnostics but not hashed: type arguments are positional, so a rename changes nothing an importer can do. A generic body's hash still changes, because its tokens change.
- Every exported function with type parameters, generic extensions included, records the Monomorphization Hash, as before. Phase 1 still reads no body except for that hash. Arity and bounds of nominal references are the type checker's job (`E4020`, `E4021`).
- `type_from_abi` turns `Parameter { index }` into the declaration's `TypeParameter` with its bound.

Rejected: hashing parameter names, as the old `Vec<String>` did; and binding parameters by name in the importer, the bug this fixes.

### D14. Error codes (autonomous)

New, at the next free codes of the type checker's range:

- `E4020` `WrongTypeArgumentCount`: a generic type or call given the wrong number of type arguments, including none for a generic type in an annotation (`val b: Box`), any for a non-generic callee, and any for a type parameter.
- `E4021` `UnsatisfiedBound`: a type argument that is not `Copy` (or `Clone`) where the type parameter is bounded by it, also in a generic body (`twice(x)` for an unbounded `x: T`).
- `E4022` `BorrowInTypeArgument`: D7, rule 1.

Widened: `E2001` (D2, D3, D5, D10); `E4006` (members of a type parameter, a receiver that does not match a generic extension); `E4010` (duplicate or built-in-named type parameters, one extension per receiver constructor); `E4013` (type arguments that cannot be inferred, a generic function as a value); `E4016` (conditional markers); `E4018` (unbounded type parameters, help suggesting a bound); `E5007` (passing to a generic parameter). Rejected: one code for every generic error. Arity, bounds and borrows each have a different fix.

### D15. Direction for `monomorphization`, recorded but not decided (autonomous)

This change does not ship generic bodies, so an importer cannot instantiate an imported generic yet: `prism check` accepts such calls, and code generation will refuse them until `monomorphization`. The direction recorded for that change, as an open decision in `docs/architecture.md` §1:

- The exporter ships each generic function's elaborated polymorphic MIR, which is self-describing because calls record paths, declared types and type arguments (D9). It is not the typed tree, so no importer reruns the borrow checker, and not the source, so no importer re-typechecks.
- Instances are better kept out of the importer's file artifact: a separate instance artifact keyed by (generic path, type arguments, the generic's MIR hash) keeps a file's artifact a function of its own content and its direct imports' ABI hashes, even when an instance calls generics of modules the importer does not import, and even with import cycles. Re-monomorphization then regenerates only instances whose generic body changed, which is the planned "Re-monomorphization on Generic Body Change" requirement. It may also take the Phase 1 body hash out of the ABI hash, since an importer's own code would no longer depend on generic bodies.
- Still to decide there: whether exported generic bodies may reference private declarations (Rust's model, which ships what they reach) or only exported ones (Swift's `@inlinable` model), rejecting polymorphic recursion (`fun <T> f(x: T) { f(Box(x)) }` has infinitely many instances; the checker accepts it), and the MIR rewrites for flattened nullables (D4).

None of this is hard to revert: it is unimplemented, and the language rules of this change do not depend on it.

### D16. A new `generics` capability (autonomous)

The language rules for type parameters, bounds, type arguments, inference, the borrow-free rule, conditional markers and generic extensions go into a new `generics` spec. The changes to existing rules go into the specs that own them: the grammar into `syntax`, the `Copy`/`Clone` tables into `ownership`, the ABI into `compilation-and-abi`, the escape into `references`, and the representation into `typed-tree` and `mir`. Rejected: putting everything into `type-system`, which is already a long catch-all.

## Risks / Trade-offs

- [Changing `Type::Struct(NominalId)` into a variant with arguments touches about 70 matches across seven crates] → The compiler finds every one. Task 2 lands the representation with empty arguments and no behaviour change before anything uses it.
- [Recording declared parameter types (D9) makes every consumer of `Call.parameters` substitute] → One accessor, `Call::parameter_type(i)`, used everywhere, and both validators check every slot against the substituted type.
- [Speculative parsing of call type arguments could misfire] → The rule is local (tokens up to the matching `>` plus one), never mutates tokens, and has scenarios for the tricky forms: `a < b`, `a < b > (c)`, `g(a < b, c > (d))`, `i<n)`, `x < y >= z`, and `f<Int>\n(x)`, where the newline rule makes `(x)` a new statement.
- [Local inference rejects some programs Kotlin accepts] → An empty `listOf()` without an expected type, or a type fixed only by a later use, is `E4013` with help to write the type argument. Widening inference later accepts more programs and breaks none.
- [The borrow-free rule (D7) rejects `Box<&T>`-style code that Rust accepts] → The help points to `&T` in signatures. An opt-in bound can relax it later.
- [Polymorphic recursion passes `prism check`] → Recorded for `monomorphization` (D15).
- [Annotations that used to be accepted are now errors: `Heap<&Int>` in a local or parameter, `Box` without arguments, `Heap<A, B>`] → They were silently wrong. The fixture suite and `examples/` are rerun (task 12), and each changed expectation is listed in the implementation notes.

## Migration Plan

Prism is unreleased, so there is no migration beyond the repository. `fun id<T>` in `crates/prism-abi/tests` and any fixture or example becomes `fun <T> id`. The fixture `ownership/non-copy-built-ins-rejected-in-a-copy-declaration--type-parameter` and the Clone counterpart change their expectation to `ok` with the modified scenarios (D6). Their unbounded-`T` replacements go under `generics/`.

## Open Questions

- Whether `prism dump-typed-tree` should show the solved type arguments of calls whose arguments were inferred differently from written ones. The proposal prints them the same way. This only affects snapshots.
- Whether the help of `E4013` for a generic callee should print the full suggested call (`make<...>()`) or only the type parameter's name. It only affects message text.
