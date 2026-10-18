# Execution Specification Delta

## ADDED Requirements

### Requirement: Instances Drop and Clone Type-Parameter Values
In an instance, a value of a type parameter's type MUST be dropped, moved and copied as a value of the type argument is: dropping it runs the argument's drop glue, a move transfers its storage, and a `clone()` under a `Clone` bound builds a value that shares no storage with the original. A value whose argument owns nothing MUST drop without running any code.

#### Scenario: Dropping and cloning strings through generic code
- **WHEN** `struct Pair<A, B>(val a: A, val b: B)`, `fun <T> pick(a: T, b: T, first: Bool): T = if (first) a else b`, `fun <T> discard(x: T) {}` and `fun <T : Clone> both(x: &T): Pair<T, T> = Pair(x.clone(), x.clone())` are declared and `fun main() { println(pick("a", "b", false)); discard("c" + "d"); val p = both(&("e" + "f")); println(p.a + p.b) }` runs under `prism run --check-allocations`
- **THEN** the program prints `b` and `efef`, reports no live allocation and no double free, and exits with status 0

#### Scenario: Generic code over structs that own strings
- **WHEN** `struct User : Clone { val name: String }`, `struct Box<T>(val v: T)` and `fun <T> unwrap(b: Box<T>): T { val (v) = b; return v }` are declared and `fun main() { val u = unwrap(Box(User("ada"))); println(u.name); val c = unwrap(Box(Box(u.clone()))); println(c.v.name) }` runs under `prism run --check-allocations`
- **THEN** the program prints `ada` twice, reports no live allocation and exits with status 0

### Requirement: Nullable Type Arguments Flatten at Run Time
An instance whose type argument for `T` is nullable MUST treat `T?` as that argument: a `T` wrapped into a `T?` is the same value, `null` included, a `T?` compared with `null` tests the argument's own tag, and `?:` on a `T` or a `T?` takes its fallback exactly when the value is `null`.

#### Scenario: Wrapping a nullable argument keeps null
- **WHEN** `fun <T> wrap(a: T): T? = a` and `fun <T> isNull(a: T?): Bool = a == null` are declared and `fun main() { println(isNull(wrap<Int?>(null))); println(isNull(wrap<Int?>(1))); println(isNull<Int>(null)); println(wrap<String?>("s") ?: "none") }` runs
- **THEN** the program prints `true`, `false`, `true` and `s`

### Requirement: Generic Default Values Run Per Instance
A default value of a generic function MUST be evaluated by an instance of its generator for the call's type arguments, with the earlier arguments the call passed.

#### Scenario: A default value of a generic function
- **WHEN** `fun <T : Copy> pad(x: T, y: T = x): T = y` is declared and `fun main() { println(pad(1)); println(pad(true, false)); println(pad('c')) }` runs
- **THEN** the program prints `1`, `false` and `c`

### Requirement: Generic Recursive Data Runs
Generic structs and enums that reach themselves through `Heap<T>` with the same type arguments MUST run through generic functions and extensions, with every box freed exactly once.

#### Scenario: A generic linked list
- **WHEN** `enum List<T> { Cons(T, Heap<List<T>>), Nil }`, `fun <T> List<T>.size(): Int = when (this) { List.Cons(_, val rest) -> 1 + rest.size(), List.Nil -> 0 }` and `fun <T> push(list: List<T>, x: T): List<T> = List.Cons(x, Heap(list))` are declared and `fun main() { val empty: List<String> = List.Nil; val names = push(push(empty, "a"), "b"); val none: List<Int> = List.Nil; val numbers = push(none, 1); println(names.size()); println(numbers.size()) }` runs under `prism run --check-allocations`
- **THEN** the program prints `2` and `1`, reports no live allocation and exits with status 0

### Requirement: Runtime Errors in an Instance Name the Generic's Source
A runtime error raised by an instance MUST be reported at the location in the generic's own file that raised it, whichever file called the instance.

#### Scenario: Division by zero in an imported generic
- **WHEN** `lib/math.prism` declares `public fun <T> ratio(x: T, a: Int, b: Int): Int = a / b` on line 2, with `a / b` starting at column 51, and `app/main.prism` imports `/lib/math` and declares `fun main() { println(ratio("x", 1, 0)) }`
- **THEN** stderr is exactly `panic: division by zero` and `  at lib/math.prism:2:51`, each followed by a newline, and the exit status is 101
