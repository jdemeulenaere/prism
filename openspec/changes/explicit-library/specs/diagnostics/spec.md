## ADDED Requirements

### Requirement: Unresolved Library Names Suggest Their Import
When a bare name (`E4001`) or a member or extension (`E4006`) is not found, and a public declaration of that name in a library module would be found after importing it, the diagnostic MUST carry one help line per such module naming its import (`import /prism/core`). An extension MUST match only when its receiver type constructor is the receiver's, or a type parameter. A name no library module declares MUST get no such help.

#### Scenario: An unimported library function
- **WHEN** a file without imports declares `fun main() { println(1) }`
- **THEN** `prism check` reports `E4001` for `println` with the help line `` `println` is declared by the library module `/prism/core`: add `import /prism/core` ``

#### Scenario: An unimported library extension
- **WHEN** a file without imports declares `fun f(xs: &List<Int>): Int = xs.sumOf { x -> x }` and `fun g(n: Int): Int = n.sumOf { x -> x }`
- **THEN** the `E4006` for `sumOf` in `f` carries help naming `import /prism/collections`, and the one in `g` carries none, since no library module declares `sumOf` on `Int`

#### Scenario: A name the library does not declare
- **WHEN** a file without imports declares `fun main() { printn(1) }`
- **THEN** `prism check` reports `E4001` for `printn` with no help naming an import
