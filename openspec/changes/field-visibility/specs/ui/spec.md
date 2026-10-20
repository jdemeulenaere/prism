## ADDED Requirements

### Requirement: Modifier Chains Are Built Only Through Modifier Functions
The fields of `Modifier` (`count`, `first` and `second`) and of `ModifierElements` (`e0` to `e7`) MUST be `private` to `/prism/compose/ui`. A program MUST build and combine chains only through `Modifier` and the modifier functions (`padding`, `size`, `clickable`, `then`, ...): constructing either struct or reading its fields outside that module MUST be `E4007`.

#### Scenario: A chain cannot be built or read by hand
- **WHEN** a file importing `/prism/compose/ui` declares `fun count(m: &Modifier): Int = m.count` and `fun empty(): Modifier = Modifier(0, Modifier.first, Modifier.second)`
- **THEN** the type checker emits `E4007` at `count`, at the construction of `Modifier`, and at `first` and `second`, each naming a private field of `prism/compose/ui`

#### Scenario: Chains built through modifier functions still compare
- **WHEN** a file importing `/prism/compose/ui`, `/prism/compose/layout` and `/prism/core` runs `fun main() { println(Modifier.padding(4.dp).size(8.dp) == Modifier.padding(4.dp).then(Modifier.size(8.dp))) }` with `--check-allocations`
- **THEN** stdout is `true`, and the exit status is 0
