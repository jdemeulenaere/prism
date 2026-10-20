## MODIFIED Requirements

### Requirement: Lists Are Built With listOf
`listOf` MUST be the ordinary library function `public fun <T> listOf(vararg elements: T): List<T> = elements` of `/prism/collections` (standard-library), typed and run as any vararg call (vararg-parameters): explicit type arguments (`listOf<Int?>(1)`), the expected type and the joined elements give `T`, each element is consumed as a `T`, a conflicting element is `E4004`, an unsolved `T` (`val xs = listOf()`) is `E4013`, and a spread gives its list back.

#### Scenario: Element types from arguments, joins and expected types
- **WHEN** `fun f(): List<Int?> { val a = listOf(1, 2, 3); val b = listOf(1, null); val c: List<String> = listOf(); val d = listOf<Double>(); return b }` is checked
- **THEN** the file typechecks successfully: `a` is a `List<Int>`, `b` a `List<Int?>`, `c` a `List<String>` and `d` a `List<Double>`

#### Scenario: Arguments are moved into the list
- **WHEN** `fun f(s: String): List<String> { val xs = listOf(s, "b"); println(s); return xs }` is checked
- **THEN** the borrow checker emits `E5001` at `println(s)`, because `listOf` consumed `s`

#### Scenario: Conflicting or missing element types rejected
- **WHEN** `fun f() { val a = listOf(1, "a") }` and `fun g() { val b = listOf() }` are checked
- **THEN** the type checker emits `E4004` at `"a"` and `E4013` for `b`, naming the type parameter `T` and suggesting `listOf<...>()` or a type annotation

#### Scenario: Spreading a list into listOf
- **WHEN** `fun f(xs: List<Int>): List<Int> = listOf(elements = *xs)` and `fun g(xs: List<Int>): List<Int> = listOf(*xs)` are checked, and `fun h(): List<Int> = listOf(elements = 1)` is checked
- **THEN** `f` and `g` typecheck, each returning the list it was given, and `h` is rejected with `E4004`, a named vararg argument being a spread
