## MODIFIED Requirements

### Requirement: List Members Are Typed as Extensions
The members of a `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST be the public extensions on `List<T>` that `/prism/core` declares (`public intrinsic val <T> List<T>.size: Int`, `public fun <T> List<T>.isEmpty(): Bool`). Each is typed by its declared receiver mode and signature, so receivers, arguments (`E4004`) and safe calls (`xs?.add(1)`) are checked as for an extension. The reading members are `isEmpty(): Bool` and `isNotEmpty(): Bool`, written in Prism over `size`, and the intrinsics `contains(element: &T): Bool` and `indexOf(element: &T): Int`.

#### Scenario: Wrong element types rejected
- **WHEN** `fun f() { var xs = listOf(1); xs.add("a"); xs.insert("b", 1) }` is checked
- **THEN** the type checker emits `E4004` for `"a"` and for the index `"b"`, while the element `1` fits

#### Scenario: Searching borrows the element
- **WHEN** `fun f(xs: &List<String>, name: String): Bool = xs.contains(name) && xs.indexOf(name) >= 0 && !xs.isEmpty()` is checked
- **THEN** the file typechecks successfully, and `name` is borrowed, not moved

### Requirement: Size and Indices Are Read Without Parentheses
A `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST have the member properties `size` of type `Int`, an intrinsic, and `indices` of type `Range` (from 0, end excluded), written in Prism as `0..<size`. Both are read without parentheses with a shared borrow of the list, as `length` of a `String` is, and `xs?.size` on a nullable list MUST have type `Int?`. Calling either (`xs.size()`) MUST be rejected with `E4004`, as calling an `Int` or a `Range` is.

#### Scenario: Reading the size
- **WHEN** `fun f(a: List<Int>, b: &List<String>, c: List<Int>?): Int { var n = a.size + b.size + (c?.size ?: 0); for (i in a.indices) n += a[i]; return n }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Calling the size rejected
- **WHEN** `fun f(xs: List<Int>): Int = xs.size()` is checked
- **THEN** the type checker emits `E4004`, saying that `Int` cannot be called

### Requirement: List Members Take Precedence Over Extensions
A call `xs.name(...)` or read `xs.name` on a `List<T>` MUST use its member of that name (`standard-library`, "Intrinsic Extensions Are Members"), after the built-in `clone()` and before any extension. A workspace extension on `List` named like a member could never be called and MUST be rejected with `E4010`. The members are `size`, `indices`, `isEmpty`, `isNotEmpty`, `contains`, `indexOf`, `add`, `insert`, `removeAt`, `removeLast`, `reverse` and `clear`, plus `clone` for a `Clone` element type.

#### Scenario: Extensions named like built-in members rejected
- **WHEN** a file declares `fun <T> List<T>.add(element: T) {}`, `fun List<Int>.size(): Int = 0` and `fun <T> List<T>.isEmpty(): Bool = false`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/core` always takes precedence

### Requirement: Lists Answer Membership Tests
`x in xs` and `x !in xs` on a `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST call its `contains` member, declared `intrinsic operator` by `/prism/core`, borrowing `x`; they MUST be accepted exactly when `contains` is (`==` on `T`).

#### Scenario: Membership in a list
- **WHEN** `fun f(names: &List<String>, n: String): Bool = n in names && "x" !in names` is checked
- **THEN** the file typechecks, each test calling the `contains` member with `n` and `"x"` borrowed, and `n` is not moved

#### Scenario: Membership in a list of a type parameter rejected
- **WHEN** `fun <T> has(xs: &List<T>, x: &T): Bool = x in xs` is checked
- **THEN** the type checker emits `E4002`, as for `xs.contains(x)`
