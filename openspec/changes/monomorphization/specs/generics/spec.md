# Generics Specification Delta

## ADDED Requirements

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
A generic struct or enum MUST NOT reach itself through its fields or payloads, `Heap<T>` included, with a type argument that holds one of its own type parameters nested in another type, since its layout, drop glue and equality would then need infinitely many types. Such a field or payload MUST be `PolymorphicRecursion` (`E4027`) at the declaration. A type that `E4023` already rejects is not reported again.

#### Scenario: A generic type reaching itself through a box with a growing argument
- **WHEN** `struct Box<T>(val v: T)` and `struct Nest<T>(val v: T, val next: Heap<Nest<Box<T>>>?)` are checked
- **THEN** the type checker emits `E4027` at the field `next`, saying that `Nest` contains `Nest<Box<T>>`

#### Scenario: A regular recursive generic type accepted
- **WHEN** `enum List<T> { Cons(T, Heap<List<T>>), Nil }` and `struct Tree<T>(val v: T, val kids: Heap<List<Tree<T>>>?)` are checked
- **THEN** the file passes `prism check`

### Requirement: Polymorphic Recursion Across Files Is Rejected When Compiled
A cycle of generic function calls with a growing type argument that passes through generics of several files, which only files importing each other can form, MUST be reported as `PolymorphicRecursionAcrossFiles` (`E6004`) when the program is compiled, naming each generic on the cycle, and the program MUST NOT run. `prism check` does not report it, since it checks each file alone.

#### Scenario: Polymorphic recursion through two files
- **WHEN** `a/x.prism` imports `/a/y` and declares `public struct Box<T>(val v: T)` and `public fun <T> ping(x: T, n: Int): Int = if (n == 0) 0 else pong(Box(x), n - 1)`, `a/y.prism` imports `/a/x` and declares `public fun <T> pong(x: T, n: Int): Int = ping(x, n)`, and `a/main.prism` imports `/a/x` and declares `fun main() { println(ping(1, 3)) }`
- **THEN** `prism check a/main.prism` passes, and `prism run a/main.prism` reports `E6004` at `pong(Box(x), n - 1)`, naming `a/x::ping` and `a/y::pong`, runs nothing and exits with status 1
