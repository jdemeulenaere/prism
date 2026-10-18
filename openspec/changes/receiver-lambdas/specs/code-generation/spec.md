# Code Generation Specification Delta

## MODIFIED Requirements

### Requirement: Symbols Derive from Declaration Paths
Every compiled function of a declaration MUST have a symbol derived from its declaration's identity alone: `<module>::<name>` for a function, `<module>::<receiver>.<name>` for an extension, where the receiver is written as the ABI writes its type constructor, `<module>::<receiver>.<name>#context(<context>)` for an extension with a context parameter, the context type written the same way, `<function symbol>#default(<parameter>)` for a default value's generator, and `<module>::<name>#init` and `<module>::<name>#get` for a top-level `val`'s initializer and accessor. Symbols MUST be unique in a program.

#### Scenario: Symbols of a file
- **WHEN** `lib/math.prism`, in package `lib`, declares `public fun Int.double(): Int = this * 2`, `public fun scale(a: Int, b: Int = 2): Int = a * b` and `public val BASE: Int = 3`
- **THEN** `prism dump-clif lib/math.prism` prints the functions `lib/math::Int.double`, `lib/math::scale`, `lib/math::scale#default(b)`, `lib/math::BASE#init` and `lib/math::BASE#get`

#### Scenario: Symbols of extensions differing by context
- **WHEN** `lib/layout.prism`, in package `lib`, declares `public struct Modifier : Copy { val fraction: Double }`, `public struct RowScope : Copy {}`, `public struct ColumnScope : Copy {}`, `context(scope: RowScope) public fun Modifier.weight(w: Double, fill: Bool = true): Modifier = Modifier(w)` and `context(scope: ColumnScope) public fun Modifier.weight(w: Double): Modifier = Modifier(w)`
- **THEN** `prism dump-clif lib/layout.prism` prints the functions `lib/layout::Modifier.weight#context(RowScope)`, `lib/layout::Modifier.weight#context(RowScope)#default(fill)` and `lib/layout::Modifier.weight#context(ColumnScope)`
