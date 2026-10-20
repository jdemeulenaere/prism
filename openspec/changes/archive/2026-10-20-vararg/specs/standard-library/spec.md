## MODIFIED Requirements

### Requirement: Intrinsic Calls Are Typed by Their Signatures
A call of an intrinsic MUST be checked as a call of its declared signature, except for those the catalog types by a rule:
- `print` and `println` (`fun <T> print(value: &T)`) take an argument with a text form (type-system, "Output Built-In Arguments"), and `println` also none;
- `contains` and `indexOf` of `List<T>` also need `==` on `T`;
- `remember` also takes keys ("Remember Takes Up to Three Keys").

#### Scenario: A signature-typed intrinsic
- **WHEN** `fun f(xs: &List<Int>): Bool = xs.isEmpty(1)` is checked
- **THEN** the type checker emits `E4004`, since `isEmpty` is declared with no parameter

#### Scenario: A rule-typed intrinsic
- **WHEN** `fun f() { println(); println(1) }` is checked
- **THEN** the file typechecks, although `println` is declared with one parameter

### Requirement: The Collections Module Declares Lists and Their Helpers
`/prism/collections` MUST declare, as public intrinsics, the members of `List<T>` (collections, "List Members Are Typed as Extensions", "Mutating Members", "Size and Indices Are Read Without Parentheses"), and, as public Prism functions, `listOf` (collections, "Lists Are Built With listOf") and the higher-order helpers (collections, "Higher-Order Helpers").

#### Scenario: Helpers are imported
- **WHEN** `import /prism/collections`, `import /prism/core` and `fun main() { val xs = listOf(1, 2, 3); println(xs.map { x -> x * 2 }.sumOf { x -> x }) }` run
- **THEN** it prints `12`

#### Scenario: listOf is written in Prism
- **WHEN** `cargo test -p prism-cli --test library_tests` checks the library, and `prism dump-typed-tree` runs on a file declaring `import /prism/collections` and `fun f(): List<Int> = listOf(1, 2)`
- **THEN** `library/prism/collections.prism` declares `public fun <T> listOf(vararg elements: T): List<T> = elements` and passes its checks, the intrinsic catalog has no `listOf`, and the body of `f` is a call of `prism/collections::listOf<Int>` whose argument is a call of `builtin varargList<Int>` with the literals `1` and `2`
