# Generics Specification

## Purpose
The generics specification defines type parameters on functions, extensions, structs and enums: their scope and bounds, what a generic body may do with a type parameter, explicit and inferred type arguments, the rule that type arguments are borrow-free, and generic types and extensions. A generic declaration is checked once, where it is declared, and runs as an instance per list of concrete type arguments the program uses; generics that would need infinitely many instances (polymorphic recursion) are rejected.

## Requirements

### Requirement: Generic Declarations
Functions, extension functions, structs and enums MUST accept type parameters. A function's type parameters MUST be in scope in its receiver, parameters, default values, return type and body. A struct's or enum's MUST be in scope in its fields or payloads. A type parameter MUST shadow a struct or enum of the same name. A duplicate type parameter, or one named like a built-in type other than `Range`, MUST be rejected with `E4010`.

#### Scenario: Generic function, struct and enum
- **WHEN** `fun <T> id(x: T): T = x`, `struct Box<T>(val v: T)` and `enum Option<T> { Some(T), None }` are declared, and `fun f(): Int = id(1)`, `fun g(b: Box<Int>): Int = b.v` and `fun h(): Option<Int> = Option.Some(2)` are checked
- **THEN** the file typechecks successfully

#### Scenario: A type parameter shadows a struct
- **WHEN** `struct T { val n: Int }` and `fun <T> id(x: T): T = x` are declared and `fun f(): String = id("a")` is checked
- **THEN** the file typechecks successfully, because `T` in `id` is its type parameter, not the struct

#### Scenario: Duplicate and built-in type parameter names rejected
- **WHEN** `fun <T, T> f(x: T): T = x` or `fun <Int> g(x: Int): Int = x` is declared
- **THEN** the type checker emits `E4010`

### Requirement: Generic Bodies Are Checked Once
A generic declaration MUST be type-checked and borrow-checked once, against its type parameters and their bounds alone, whether or not anything instantiates it. An instantiation MUST NOT be checked against the generic body, so a use is accepted or rejected by the generic signature alone.

#### Scenario: Error in a generic body without any instantiation
- **WHEN** `fun <T> size(x: &T): Int = x.length` is declared and nothing calls it
- **THEN** the type checker emits `E4006`, because a type parameter has no members

#### Scenario: One body for every instantiation
- **WHEN** `fun <T> keep(x: T): T { val y = x; return y }` is declared, and `fun f(): Int = keep(1)` and `fun g(s: String): String = keep(s)` are checked
- **THEN** the file passes `prism check`

### Requirement: Unbounded Type Parameters Are Move-Only
In a generic body, a value of an unbounded type parameter `T` MUST be neither `Copy` nor `Clone`. It MAY be moved, borrowed, dropped, passed on, returned, wrapped in `T?` or `Heap<T>` and stored in a generic struct. Calling `clone()` on it MUST be `E4018`, accessing a member `E4006`, and applying `==`, `!=`, an ordering or an arithmetic operator to it `E4002`.

#### Scenario: Using an unbounded value twice
- **WHEN** `struct Pair<A, B>(val a: A, val b: B)` is declared and `fun <T> dup(x: T): Pair<T, T> = Pair(x, x)` is checked
- **THEN** the borrow checker emits `E5001` at the second `x`, because a type-parameter value moves

#### Scenario: Cloning and comparing an unbounded value rejected
- **WHEN** `fun <T> c(x: &T): T = x.clone()` or `fun <T> same(a: &T, b: &T): Bool = a == b` is checked
- **THEN** the type checker emits `E4018` for `c`, with help suggesting the bound `<T : Clone>`, and `E4002` for `same`

#### Scenario: Comparing type-parameter values names the operator
- **WHEN** `fun <T> same(a: &T, b: &T): Bool = a == b` and `fun <T> differ(a: &T, b: &T): Bool = a != b` are checked
- **THEN** the type checker emits `E4002` for each, saying that operator `==` (or `!=`) cannot be applied to `T` and `T`, with help saying that type parameters have no equality until traits exist

#### Scenario: Moving out of a borrowed type parameter rejected
- **WHEN** `fun <T> get(x: &T): T = x` is checked
- **THEN** the type checker emits `E4017`, because only a `Copy` value is copied out of a borrow

### Requirement: Copy and Clone Bounds
A function type parameter MAY be bounded by `Copy` or `Clone` (`<T : Copy>`). A type parameter bounded by `Copy` MUST be `Copy` and `Clone`, and one bounded by `Clone` MUST be `Clone`. A type argument that does not satisfy its parameter's bound MUST be rejected with `UnsatisfiedBound` (`E4021`), naming the argument, the parameter and the bound, including in a generic body that passes on an insufficiently bounded type parameter.

#### Scenario: Bounded type parameters copy and clone
- **WHEN** `struct Pair<A, B>(val a: A, val b: B)` is declared and `fun <T : Copy> dup(x: T): Pair<T, T> = Pair(x, x)` and `fun <T : Clone> c(x: &T): T = x.clone()` are checked
- **THEN** the file passes `prism check`

#### Scenario: Unsatisfied bound rejected
- **WHEN** `fun <T : Copy> dup(x: T): T = x` is declared, and `fun f(s: String): String = dup(s)`, `fun g(s: String): String = dup<String>(s)` or `fun <U> h(x: U): U = dup(x)` is checked
- **THEN** the type checker emits `E4021` for each, saying that `String` (or `U`) is not `Copy`

### Requirement: Explicit Type Arguments
A call of a generic function, extension, struct constructor or payload variant MAY give its type arguments explicitly, one per type parameter, in declaration order. An extension's type arguments MUST include the type parameters that appear in its receiver. A wrong number of type arguments, or type arguments for a callee that is not generic, MUST be rejected with `WrongTypeArgumentCount` (`E4020`).

#### Scenario: Explicit type arguments
- **WHEN** `fun <T> id(x: T): T = x` and `struct Box<T>(val v: T)` are declared and `fun f(): Int? = id<Int?>(1)` and `fun g(): Box<Int?> = Box<Int?>(null)` are checked
- **THEN** the file typechecks successfully

#### Scenario: Wrong number of type arguments rejected
- **WHEN** `fun <T> id(x: T): T = x` and `fun one(): Int = 1` are declared and `fun f(): Int = id<Int, Int>(1)` or `fun g(): Int = one<Int>()` is checked
- **THEN** the type checker emits `E4020` for each, naming the expected and the given number of type arguments

### Requirement: Type Argument Inference
Without explicit type arguments, a call MUST infer them from the receiver, the expected type and the arguments. The expected type MUST fix the type parameters of the return type (an expected `X?` fixes a declared `T?` only as a fallback). Arguments MUST give lower bounds, which are joined. Closure arguments MUST be checked last, and their bodies MAY solve return type parameters. An unsolved type parameter MUST be `CannotInferType` (`E4013`).

#### Scenario: Inference from arguments, joins and the expected type
- **WHEN** `fun <T> pick(a: T, b: T): T = a`, `fun <T> id(x: T): T = x` and `struct Box<T>(val v: T)` are declared and `fun f(): Int? { val n = pick(1, null); return n }` and `fun g(): Box<Int?> { val b: Box<Int?> = Box(null); return b }` are checked
- **THEN** the file typechecks successfully: `pick` is instantiated with `Int?`, the join of `Int` and `null`, and `Box` with `Int?` from the expected type

#### Scenario: Expected types reach a fallback and a borrowed argument
- **WHEN** `struct Box<T>(val v: T)` and `fun take(b: &Box<Int?>) {}` are declared and `fun f(m: Box<Int?>?): Box<Int?> { val b: Box<Int?> = m ?: Box(null); return b }` and `fun g() { take(Box(null)) }` are checked
- **THEN** the file typechecks successfully: the fallback of `?:` is checked against the type the `?:` is, and an argument passed to a borrow against the borrow's referent, so `Box` is instantiated with `Int?` in both

#### Scenario: Inference from closure results
- **WHEN** `fun <T> remember(calc: () -> T): T = calc()` and `fun <T, R> map(x: T, f: (T) -> R): R = f(x)` are declared and `fun f(): Int { val n = remember { 0 }; return n }` and `fun g(): Int { val n = map("a") { s -> 1 }; return n }` are checked
- **THEN** the file typechecks successfully, with `remember` instantiated with `Int`, and `map` with `String` and `Int`

#### Scenario: Conflicting arguments rejected
- **WHEN** `fun <T> pick(a: T, b: T): T = a` is declared and `fun f(): Int = pick(1, "a")` is checked
- **THEN** the type checker emits `E4004` at `"a"`

#### Scenario: Type argument that cannot be inferred
- **WHEN** `enum Option<T> { Some(T), None }`, `fun <T> none(): Option<T> = Option.None` and `fun <T> keep(x: T): T = x` are declared, and `fun f() { val o = none() }`, `fun g() { val o = Option.None }` or `fun h() { val k = keep }` is checked
- **THEN** the type checker emits `E4013` for each, naming the type parameter `T` and suggesting explicit type arguments or a type annotation, while `fun k() { val o: Option<Int> = none() }` typechecks successfully

### Requirement: Type Arguments Are Borrow-Free
A type argument, written or inferred, MUST NOT contain a reference outside a function type, wherever it appears: in a call, an annotation, a generic struct or enum type, or `Heap<T>`. Such an argument MUST be rejected with `BorrowInTypeArgument` (`E4022`). Inference MUST NOT infer a reference for a by-value type parameter, and MUST take the referent instead, which is then copied out or rejected as a move out of a borrow.

#### Scenario: Borrows as type arguments rejected
- **WHEN** `fun <T> id(x: T): T = x`, `fun <T> apply(f: (T) -> Unit) {}` and `struct Box<T>(val v: T)` are declared, and `fun f(a: Int): Int { val r = id<&Int>(&a); return 1 }`, `fun g(b: Box<&Int>) {}`, `fun h(b: Heap<&Int>) {}` or `fun k(s: (&Int) -> Unit) { apply(s) }` is checked
- **THEN** the type checker emits `E4022` for each, with help suggesting `&T` in the generic signature

#### Scenario: A borrowed argument instantiates its referent
- **WHEN** `fun <T> id(x: T): T = x` is declared, and `fun f(a: Int): Int = id(&a)` and `fun g(s: String): String = id(&s)` are checked
- **THEN** `f` typechecks successfully, with `id` instantiated with `Int` and `a` copied out of the borrow, and the type checker emits `E4017` for `g`

#### Scenario: A closure's borrowed result instantiates its referent
- **WHEN** `fun <T> first(a: &T): &T = a` is declared, and `fun main() { println(listOf(1, 2).map { it }.sumOf { it }) }`, `fun main() { println(listOf(listOf(1, 2), listOf(3)).map { row -> row.map { first(it) }.sumOf { it } }.sumOf { it }) }` and `fun g(): List<String> = listOf("a").map { it }` are checked
- **THEN** both `main`s run, printing `3` and `6`: `map`'s result type parameter is instantiated with `Int`, the referent of each closure's `&Int` result, which is copied out of the borrow without checking the closure body again; and the type checker emits `E4017` for `g`, with help suggesting `it.clone()`

#### Scenario: Borrows in a generic signature accepted
- **WHEN** `fun <T> first(a: &T, b: &T): &T = a` is declared and `fun f(x: String, y: String): Int { val r = first(&x, &y); return 1 }` is checked
- **THEN** the file passes `prism check`

### Requirement: Generic Types Are Distinct and Invariant
A generic struct or enum MUST be written with exactly one type argument per type parameter in annotations, else `E4020`. Its instantiations MUST be distinct types, equal only with equal type arguments, and invariant in each one. Its fields and payloads MUST have the declared types with the type arguments substituted. A nullable type parameter instantiated with a nullable type MUST be that type (`T?` with `T = String?` is `String?`).

#### Scenario: Instantiations are distinct and invariant
- **WHEN** `struct Box<T>(val v: T)` is declared and `fun f(b: Box<Int>): Box<String> = b` or `fun g(b: Box<Int>): Box<Int?> = b` is checked
- **THEN** the type checker emits `E4005` for each

#### Scenario: Fields are substituted
- **WHEN** `struct Box<T>(val v: T)` is declared and `fun f(b: &Box<Int>): Int = b.v + 1` is checked
- **THEN** the file typechecks successfully, `b.v` being an `Int`

#### Scenario: Missing or extra type arguments in an annotation rejected
- **WHEN** `struct Box<T>(val v: T)` is declared and `fun f(b: Box) {}`, `fun g(b: Box<Int, Int>) {}`, `fun h(b: Heap<Int, Int>) {}` or `fun <T> k(x: T<Int>) {}` is checked
- **THEN** the type checker emits `E4020` for each

#### Scenario: Nullable type parameters flatten
- **WHEN** `fun <T> orNull(x: T, keep: Bool): T? = if (keep) x else null` is declared and `fun f(s: String?): String? = orNull(s, true)` is checked
- **THEN** the file typechecks successfully, `orNull` returning `String?`

#### Scenario: A when over a generic enum
- **WHEN** `enum Option<T> { Some(T), None }` is declared and `fun f(o: Option<Int>): Int = when (o) { Option.None -> 0, else -> 1 }` is checked
- **THEN** the file typechecks successfully

### Requirement: Generic Extension Functions
An extension's receiver MAY carry type arguments, written with the extension's type parameters or concrete types. Extensions MUST be keyed by the receiver's type constructor and name, so one file declaring two extensions with the same name on one constructor MUST be `E4010`. A call MUST match the receiver against the declared receiver type, and a receiver that does not match MUST be `E4006`.

#### Scenario: Generic and concrete receivers
- **WHEN** `struct Box<T>(var v: T)` is declared with `fun <T> Box<T>.get(): &T = &v`, `mut fun <T> Box<T>.put(x: T) { v = x }` and `fun Box<Int>.inc(): Int = v + 1`, and `fun f(b: Box<Int>): Int { var c = b; c.put(2); val n: Int = c.get(); return c.inc() + n }` is checked
- **THEN** the file passes `prism check`

#### Scenario: Receiver that does not match rejected
- **WHEN** `struct Box<T>(val v: T)` and `fun Box<Int>.inc(): Int = v + 1` are declared and `fun f(b: Box<String>): Int = b.inc()` is checked
- **THEN** the type checker emits `E4006`, with help naming the declared receiver `Box<Int>`

#### Scenario: Two extensions on one constructor rejected
- **WHEN** `struct Box<T>(val v: T)`, `fun Box<Int>.size(): Int = 1` and `fun <T> Box<T>.size(): Int = 2` are declared in one file
- **THEN** the type checker emits `E4010`

### Requirement: Imported Generic Declarations
A file MUST be able to call an imported generic function or extension, and construct, annotate and match an imported generic struct or enum, with the type parameters, bounds and markers recorded in the import's ABI, inferring type arguments as for its own declarations.

#### Scenario: Using an imported generic API
- **WHEN** `lib/box.prism` declares `public struct Box<T> : Copy { val v: T }`, `public fun <T : Copy> twice(x: T): Box<T> = Box(x)` and `public fun <T> Box<T>.get(): &T = &v`, and `app/main.prism` imports `/lib/box` and declares `fun f(): Int { val b = twice(3); val c = b; return b.get() + c.v }` and `fun g(s: String): Box<String> = twice(s)`
- **THEN** `prism check app/main.prism` accepts `f`, where `Box<Int>` is copied, and emits `E4021` for `g`

### Requirement: Generic Code Runs as Instances
A call of a generic function, extension or default value with concrete type arguments MUST run the generic's checked body with those arguments substituted for its type parameters, whichever file declares the generic and whichever file calls it. An instance MUST NOT be type-checked or borrow-checked again, and MUST behave as the body would for a non-generic declaration of the substituted types.

#### Scenario: One generic body run at several types
- **WHEN** `struct Point : Copy { val x: Int }` and `fun <T> pick(a: T, b: T, first: Bool): T = if (first) a else b` are declared and `fun main() { println(pick(1, 2, false)); println(pick("a", "b", true)); println(pick(Point(3), Point(4), true).x) }` runs
- **THEN** the program prints `2`, `a` and `3`, and `prism run --check-allocations` reports no live allocation

#### Scenario: An imported generic API runs
- **WHEN** `lib/box.prism` declares `public struct Box<T>(val v: T)`, `public fun <T> boxed(x: T): Box<T> = Box(x)` and `public fun <T> Box<T>.get(): &T = &v`, and `app/main.prism` imports `/lib/box` and declares `fun main() { val b = boxed("hi"); println(b.get()); println(boxed(2).v) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `hi` and `2` and exits with status 0

### Requirement: Elvis on a Type Parameter Tests for Null
A `?:` whose left operand's type is a type parameter `T` MUST evaluate its fallback exactly when an instance's argument for `T` is nullable and the left operand is `null`, as it does for a `T?` left operand. In an instance whose argument for `T` is not nullable, the fallback MUST NOT be evaluated.

#### Scenario: Elvis on a nullable instantiation
- **WHEN** `fun <T> orElse(a: T, c: T): T = a ?: c` is declared and `fun main() { println(orElse<Int?>(null, 3) ?: 0); println(orElse<Int?>(4, 3) ?: 0); println(orElse(1, 2)) }` runs
- **THEN** the program prints `3`, `4` and `1`

### Requirement: Polymorphic Recursion Is Rejected
A generic function MUST NOT reach a call of itself, directly or through other generic functions of its file, in which a type argument holds its own type parameter nested in another type, since that needs infinitely many instances. Such a call MUST be `PolymorphicRecursion` (`E4027`), whether or not anything instantiates the function. Wrapping a type parameter in `?` alone does not grow it, since nullable types flatten.

#### Scenario: A function instantiating itself with a growing argument
- **WHEN** `struct Box<T>(val v: T)` is declared and `fun <T> nest(x: T, n: Int): Int = if (n == 0) 0 else nest(Box(x), n - 1)` is checked
- **THEN** the type checker emits `E4027` at `nest(Box(x), n - 1)`, saying that `nest` instantiates itself with `Box<T>` for `T`

#### Scenario: Growth through another generic function
- **WHEN** `struct Box<T>(val v: T)`, `fun <T> ping(x: T, n: Int): Int = if (n == 0) 0 else pong(Box(x), n - 1)` and `fun <T> pong(x: T, n: Int): Int = ping(x, n)` are checked
- **THEN** the type checker emits `E4027` at `pong(Box(x), n - 1)`, naming the cycle through `ping` and `pong`

#### Scenario: Recursion without growth accepted
- **WHEN** `fun <T> count(x: &T, n: Int): Int = if (n == 0) 0 else count(x, n - 1)` and `fun <T> deep(x: T?, n: Int): Int = if (n == 0) 0 else deep<T?>(x, n - 1)` are checked
- **THEN** the file passes `prism check`

### Requirement: Non-Regular Generic Types Are Rejected
A generic struct or enum MUST NOT reach itself through its fields or payloads, `Heap<T>` and `List<T>` included, with a type argument that holds one of its own type parameters nested in another type, since its layout, drop glue and equality would then need infinitely many types. Such a field or payload MUST be `PolymorphicRecursion` (`E4027`) at the declaration. A type that `E4023` already rejects is not reported again.

#### Scenario: A generic type reaching itself through a box with a growing argument
- **WHEN** `struct Box<T>(val v: T)` and `struct Nest<T>(val v: T, val next: Heap<Nest<Box<T>>>?)` are checked
- **THEN** the type checker emits `E4027` at the field `next`, saying that `Nest` contains `Nest<Box<T>>`

#### Scenario: A generic type reaching itself through a list with a growing argument
- **WHEN** `struct Box<T>(val v: T)` and `struct Nest<T>(val v: T, val more: List<Nest<Box<T>>>)` are checked
- **THEN** the type checker emits `E4027` at the field `more`, saying that `Nest` contains `Nest<Box<T>>`

#### Scenario: A regular recursive generic type accepted
- **WHEN** `enum Chain<T> { Cons(T, Heap<Chain<T>>), Nil }`, `struct Tree<T>(val v: T, val kids: Heap<Chain<Tree<T>>>?)` and `struct Forest<T>(val v: T, val trees: List<Forest<T>>)` are checked
- **THEN** the file passes `prism check`

### Requirement: Polymorphic Recursion Across Files Is Rejected
A cycle of generic function calls with a growing type argument that passes through generics of several files, which only files importing each other can form, MUST be reported as `PolymorphicRecursionAcrossFiles` (`E6004`) by `prism check` and `prism run` of any program holding the files of the cycle, whether or not anything instantiates them and whether or not their verdicts are reused, naming each generic on the cycle. The program MUST NOT run.

#### Scenario: Polymorphic recursion through two files
- **WHEN** `a/x.prism` imports `/a/y` and declares `public struct Box<T>(val v: T)` and `public fun <T> ping(x: T, n: Int): Int = if (n == 0) 0 else pong(Box(x), n - 1)`, `a/y.prism` imports `/a/x` and declares `public fun <T> pong(x: T, n: Int): Int = ping(x, n)`, and `a/main.prism` imports `/a/x` and declares `fun main() { println(ping(1, 3)) }`
- **THEN** `prism check a/main.prism` and `prism run a/main.prism` both report `E6004` at `pong(Box(x), n - 1)`, naming `a/x::ping` and `a/y::pong`, and exit with status 1, and the run runs nothing

#### Scenario: Polymorphic recursion found from reused verdicts
- **WHEN** `prism check a/x.prism` runs twice with the cache enabled on the files `a/x.prism` and `a/y.prism` of that scenario
- **THEN** both runs report the same `E6004` and exit with status 1, and the second writes no entry, since each file alone passes Phase 2 and both verdicts are reused

## Design Rationale
Decisions from `generics` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Checked once, against bounds alone.** A generic body is type- and borrow-checked where it is declared, and a use is checked against the signature alone, so no error ever comes from an instantiation, unlike C++ templates. Type parameters flow unchanged into the typed tree and MIR, so monomorphization only substitutes. `monomorphization` compiles the instances, from the checked body alone (below).
- **Nullable type parameters flatten.** `T?` with `T = String?` is `String?`, Kotlin's meaning and the existing `T??` rule; monomorphization rewrites the wraps a nullable instantiation makes redundant. An expected `X?` therefore only suggests `T = X` for a declared `T?`, used when no argument bounds `T`. Rejected: Swift's nested optionals, which would surprise Kotlin users writing `State<String?>`.
- **Only `Copy` and `Clone` bound a type parameter, and only on functions; an unbounded one is move-only.** It can be moved, borrowed, dropped, wrapped and stored, but not copied, cloned (`E4018`), accessed (`E4006`) or compared (`E4002`). `==` stays out because equality is bound to become a trait, and adding a bound later would break every body relying on it. Struct and enum bounds would need a well-formedness check at every annotation, and conditional markers (`ownership`) cover what Milestone 2 needs.
- **Type arguments are borrow-free.** A type argument, written or inferred, cannot contain a reference outside a function type (`E4022`), and inference takes a borrowed argument's referent for a by-value `T`. With the escape rule of `references` (no function value that may hold a borrow passed to a generic slot; since `escaping-closures` a capturing closure passed there owns its captures), a value of a type-parameter type carries no loans, so elision on the declared signature is exact and no body needs rechecking per instantiation. This mirrors Swift's escapable-by-default generics. Rejected: elision after substitution (errors depending on callers), checking the body with `T` possibly borrowed (which rejects `fun <T> Team.choose(x: T): T` even for `T = Int`), and forbidding function types as type arguments. An opt-in bound for borrowed instantiations would only accept more programs.
- **Inference is local and bidirectional.** Explicit arguments win; otherwise the receiver and the expected type give exact solutions, written arguments give lower bounds that are joined (`pick(1, null)` is `Int?`), closures are checked last, against their declared function type, and their bodies may solve return parameters (`remember { 0 }`). References are stripped only from an argument itself, not inside it, so `apply(s)` with `s: (&Int) -> Unit` is still `E4022`. Conflicting bounds are reported by fitting the breaking argument (`E4004`), and anything unsolved is `E4013`. The expected type reaches a call through `if`, `when`, `?:` fallbacks, returns, closures and the referent of a borrowed parameter (review). Rejected: Kotlin-style constraint solving across statements and Hindley-Milner, which make inference depend on later uses; `take(Box(null), 1)` for `fun <T> take(b: Box<T>, c: T)` is therefore rejected, since `Box(null)` is inferred before `T` is known.
- **Generic types are invariant; `Heap<T>` stays built in.** A value type's layout depends on its arguments, so covariance would hide a representation change behind assignability (the `Heap<Int>` versus `Heap<Int?>` problem of `typed-tree`). `Heap<T>` keeps its own type, auto-deref and projection, joins the arity and borrow-free rules, and is invariant too (`heap-boxes`, D8 in `type-system`).
- **Extensions are keyed by the receiver's type constructor.** `fun Box<Int>.sum()` and `fun <T> Box<T>.sum()` in one file are `E4010`, so declaration paths stay unique, and a call matches the declared receiver at the first looked-through layer whose constructor has the extension (`E4006` when it does not match). Rejected: Rust-style `impl<T> Box<T>` blocks, which do not fit extension-only methods.
- **Scopes.** A type parameter resolves before the file's and imports' types, so it may shadow a struct; names fixed by the lowering (`Int`, `String`, `Heap`, ...) and duplicates are `E4010`, and unknown names stay opaque until `E4001` lands.

Decisions from `monomorphization` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **An instance is the checked body, substituted** (D6). Since a body is checked against bounds alone and type arguments are borrow-free, substitution needs no recheck: an instance is never type- or borrow-checked again, and no error comes from an instantiation. What instances are, where they live and how they are named is in `mir` and `code-generation`.
- **`?:` on a bare `T` tests for null, as in Kotlin** (D7, BREAKING). The checker treats `T` as non-nullable, so `a ?: c` with `a: T` used to lower to `a` alone. With flattening, `val w: T? = a; w ?: c` already took the fallback for `orElse<Int?>(null, 3)`, so code differing only by an intermediate `T?` binding would have differed in meaning. The left side is wrapped into a `T?` temporary (`mir`), whose wrap an instance with a non-nullable argument always fills and one with a nullable argument flattens away; the typing is unchanged (`T`). `x == null` on a bare `T` stays `E4002`, and `?.` on a `T` gives the same result either way, so `?:` was the one place assuming non-null. Rejected: never taking the fallback (inconsistent with flattening), rejecting `?:` on a bare `T` (a common Kotlin idiom), and making unbounded `T` non-nullable (would break `State<String?>`). No accepted program changed meaning, since no generic call ran before.
- **Polymorphic recursion is a growing cycle, rejected at the declaration** (D8). The instantiation graph has a node per (generic, type parameter); a call of `g` with `A1..An` inside `f` adds an edge from each of `f`'s parameters occurring in `Aj` to `g`'s `j`th, growing unless `Aj` is the parameter itself or its nullable (flattening keeps `T??` at `T?`). Calls in default values and closures count as the generic's. A strongly connected component holding a growing edge is `E4027`, found by Tarjan's components and then a shortest path back from the growing edge (`prism_typed_tree::instantiation`), linear in the edges, and it is what lets instance collection terminate without a depth limit (Go's "instantiation cycle" check). It runs whether or not anything instantiates the generic, keeping "no error from an instantiation". Messages name each generic once per visit, so a cycle between two parameters of one generic reads as that generic instantiating itself (review). Rejected: a depth limit during instantiation (Rust's `recursion_limit`: late, arbitrary, and only once called); run-time type passing or boxing (Swift, OCaml), which is neither monomorphic nor zero-cost; rejecting every recursion between generics.
- **Non-regular generic types are rejected by the same graph** (D9). `struct Nest<T>(val next: Heap<Nest<Box<T>>>?)` has a finite layout, so `E4023` accepts it, but its glue would need `Nest<Box<T>>`, `Nest<Box<Box<T>>>` and so on. Every application of a generic struct or enum in a field or payload adds edges, through `Heap`, nullables and other generic types but not function types, whose glue does not depend on them. A cycle is reported at the growing field when it is the file's own, otherwise at the first own field on it, against the full type table, which holds every reachable type of other modules; a cycle through other modules' types alone is theirs to report, and a type `E4023` reported is skipped. Rejected: leaving it to instance collection, which would loop or need a limit in glue generation.
- **Across files, `E6004` when compiled** (D8). A growing cycle through generics of several files needs an import cycle, and no single file's check sees the other bodies (the single-file compilation units invariant), so `prism run` finds it over every record before collecting instances, and `prism check` did not see it until `instance-cache` (below).

Decisions from `collections` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Lists are generic slots and an indirection** (D2, D11). `List<T>` reaches its element type the way `Heap<T>` does, so `E4027` looks through it (`struct Nest<T> { val inner: List<Nest<List<T>>> }` would need infinitely many glue instances) while `E4023` does not follow it; and `add(element: T)` is a parameter of a type-parameter type, so a closure stored in a list owns its captures and one holding a borrow is `E5007`. A user `enum List<T>` is now `E4010`, so the examples of this spec name it `Chain<T>`.

Decisions from `instance-cache` (planned autonomously on 2026-10-17, implemented and reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **`E6004` stays, and `prism check` reports it** (D7). Every file's check entry holds a summary of its generics' calls, so a whole-program step over the summaries finds a growing cycle across files, also when every verdict is reused, in `prism check` as in `prism run`. It keeps its code rather than becoming a cross-file `E4027`: it is the same mistake, but found by a program-level step over several files' summaries rather than by one file's type checker, and its code range says which stage reports it.
