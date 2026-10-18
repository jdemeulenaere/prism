# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Receivers and Contexts in the ABI
Phase 1 MUST record whether a function type in an exported signature has a receiver, and whether that receiver is mutable, as part of its structured type, and MUST record an exported function's context type, bound like any signature type. The ABI hash MUST cover both, and the order of exported symbols MUST take the context into account, so that reordering declarations never changes the hash.

#### Scenario: A receiver function type round-trips
- **WHEN** `layout/column.prism` exports `public struct ColumnScope : Copy {}` and `public fun column(content: ColumnScope.() -> Unit): Unit { content(ColumnScope()) }`, and a file importing `/layout/column` runs `column { }`
- **THEN** the ABI of `layout/column` records the parameter type of `content` as a function type with the shared receiver `ColumnScope` from module `layout/column`, and the importer's closure is checked with that receiver

#### Scenario: Receivers and contexts change the ABI hash
- **WHEN** `public fun column(content: ColumnScope.() -> Unit): Unit` becomes `public fun column(content: (&ColumnScope) -> Unit): Unit` or `public fun column(content: mut ColumnScope.() -> Unit): Unit`, or `context(scope: RowScope) public fun Modifier.weight(w: Double): Modifier` becomes `context(scope: ColumnScope) public fun Modifier.weight(w: Double): Modifier`
- **THEN** the exported ABI hash changes

#### Scenario: Extensions differing by context in the ABI
- **WHEN** a file exports `context(scope: RowScope) public fun Modifier.weight(w: Double): Modifier` and `context(scope: ColumnScope) public fun Modifier.weight(w: Double): Modifier`, and the two declarations are reordered
- **THEN** the ABI records two distinct functions, and the exported ABI hash is the same in both orders
