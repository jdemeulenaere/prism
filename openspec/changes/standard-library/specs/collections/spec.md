# Collections Delta

## ADDED Requirements

### Requirement: Higher-Order Helpers
`/prism/collections` MUST declare public extensions on `List<T>` written in Prism over the members, found as prelude extensions (`standard-library`, "Prelude Extensions Are Found After Imported Ones"). Their function arguments MUST NOT be composable contexts and MUST NOT escape. Each MUST borrow its list, except `filter`, which consumes it.

#### Scenario: Helpers over a list of structs
- **WHEN** `struct Todo { val title: String, var done: Bool }` is declared and `fun f(todos: &List<Todo>): String = todos.joinToString { t -> t.title.clone() }` and `fun g(todos: &List<Todo>): Int = todos.count { t -> t.done }` are checked
- **THEN** the file typechecks, and each helper borrows the list

### Requirement: Visiting Helpers
`forEach` and `forEachIndexed` MUST call their action on each element in order, with its index for `forEachIndexed`:

| Helper | Signature |
|---|---|
| `forEach` | `fun <T> List<T>.forEach(action: (&T) -> Unit)` |
| `forEachIndexed` | `fun <T> List<T>.forEachIndexed(action: (Int, &T) -> Unit)` |

#### Scenario: Visiting with indices
- **WHEN** `fun main() { val xs = listOf(3, 1, 4, 1, 5); xs.forEachIndexed { i, x -> print(i * x) }; println() }` runs
- **THEN** it prints `018320` and a newline

### Requirement: Testing Helpers
`any`, `all` and `none` (`fun <T> List<T>.any(predicate: (&T) -> Bool): Bool`) MUST say whether some, every or no element satisfies `predicate`, calling it in order and stopping at the first element that decides. `count` MUST give the number of satisfying elements, and `indexOfFirst` the index of the first one, or `-1`; both take a `predicate: (&T) -> Bool` and return `Int`.

#### Scenario: any stops early
- **WHEN** `fun main() { val xs = listOf(1, 5, 9); println(xs.any { x -> print(x); x > 4 }) }` runs
- **THEN** it prints `15true`

#### Scenario: Counting and finding
- **WHEN** `fun main() { val xs = listOf(3, 1, 4, 1, 5); println(xs.count { x -> x == 1 }); println(xs.indexOfFirst { x -> x == 1 }); println(xs.all { x -> x > 0 }); println(xs.none { x -> x > 5 }) }` runs
- **THEN** it prints `2`, `1`, `true` and `true`, each on its own line

### Requirement: Mapping and Folding Helpers
These helpers MUST visit the elements in order:

| Helper | Signature |
|---|---|
| `map` | `fun <T, R> List<T>.map(transform: (&T) -> R): List<R>` |
| `fold` | `fun <T, R> List<T>.fold(initial: R, operation: (R, &T) -> R): R` |
| `sumOf` | `fun <T> List<T>.sumOf(selector: (&T) -> Int): Int` |
| `joinToString` | `fun <T> List<T>.joinToString(separator: &String = ", ", transform: (&T) -> String): String` |

`sumOf` overflowing an `Int` MUST be a runtime error.

#### Scenario: Running the mapping helpers
- **WHEN** `fun main() { val xs = listOf(3, 1, 4, 1, 5); println(xs.map { x -> x * 10 }.joinToString(separator = "-") { x -> x.toString() }); println(xs.fold(0) { a, x -> a * 10 + x }); println(xs.sumOf { x -> x }) }` runs with `--check-allocations`
- **THEN** it prints `30-10-40-10-50`, `31415` and `14`, each on its own line, and frees every allocation

### Requirement: Filtering Consumes the List
`consuming fun <T> List<T>.filter(predicate: (&T) -> Bool): List<T>` MUST give the elements that satisfy `predicate`, in order, moved out of the consumed list without being cloned, and MUST drop the others.

#### Scenario: filter consumes its list
- **WHEN** `fun f(): List<String> { val names = listOf("a", "bb", "c"); val short = names.filter { n -> n.length == 1 }; println(names.size); return short }` is checked
- **THEN** the borrow checker emits `E5001` at `names.size`, since `filter` consumed `names`

#### Scenario: Filtering strings frees the rest
- **WHEN** `fun main() { val names = listOf("a" + "", "bb" + "", "c" + ""); val short = names.filter { n -> n.length == 1 }; println(short.joinToString { n -> n.clone() }) }` runs with `--check-allocations`
- **THEN** it prints `a, c`, and every string, kept or dropped, is freed exactly once

## MODIFIED Requirements

### Requirement: List Members Are Typed as Extensions
A `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST have member functions and properties declared as public intrinsic extensions on `List<T>` in `/prism/collections` (`public intrinsic fun <T> List<T>.isEmpty(): Bool`, `public intrinsic val <T> List<T>.size: Int`), each typed by its declared receiver mode and signature, so that receivers, arguments (`E4004`) and safe calls (`xs?.add(1)`) are checked as for an extension. The reading members are `fun isEmpty(): Bool`, `fun isNotEmpty(): Bool`, `fun contains(element: &T): Bool` and `fun indexOf(element: &T): Int`.

#### Scenario: Wrong element types rejected
- **WHEN** `fun f() { var xs = listOf(1); xs.add("a"); xs.insert("b", 1) }` is checked
- **THEN** the type checker emits `E4004` for `"a"` and for the index `"b"`, while the element `1` fits

#### Scenario: Searching borrows the element
- **WHEN** `fun f(xs: &List<String>, name: String): Bool = xs.contains(name) && xs.indexOf(name) >= 0 && !xs.isEmpty()` is checked
- **THEN** the file typechecks successfully, and `name` is borrowed, not moved

### Requirement: List Members Take Precedence Over Extensions
A call `xs.name(...)` or read `xs.name` on a `List<T>` MUST use the member of that name when there is one, after the built-in `clone()` and before any extension, since the members are intrinsic extensions (`standard-library`, "Intrinsic Extensions Are Members"). A workspace extension on `List` named like a member (`size`, `indices`, `isEmpty`, `isNotEmpty`, `contains`, `indexOf`, `add`, `insert`, `removeAt`, `removeLast`, `reverse`, `clear`, or `clone` when its element type is `Clone`) could never be called, so it MUST be rejected with `E4010`.

#### Scenario: Extensions named like built-in members rejected
- **WHEN** a file declares `fun <T> List<T>.add(element: T) {}` and `fun List<Int>.size(): Int = 0`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/collections` always takes precedence

## REMOVED Requirements

### Requirement: listOf Is Found Last
**Reason**: `listOf` is a declaration of the prelude module `/prism/collections`, so its shadowing is the prelude's rule for every name (`standard-library`, "Prelude Names Are Found Last"), and naming it without a call is `E4013` (a generic function used as a value).
**Migration**: None for programs: a declaration named `listOf` still shadows it.
