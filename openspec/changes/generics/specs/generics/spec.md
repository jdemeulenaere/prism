# Generics Specification Delta

## Purpose

The generics specification defines type parameters on functions, extensions, structs and enums: their scope and bounds, what a generic body may do with a type parameter, explicit and inferred type arguments, the rule that type arguments are borrow-free, and generic types and extensions. A generic declaration is checked once, where it is declared; code generation for its instantiations is planned for `monomorphization`.

## ADDED Requirements

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
