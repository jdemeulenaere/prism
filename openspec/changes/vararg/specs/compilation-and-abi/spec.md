## ADDED Requirements

### Requirement: Vararg Parameters in the ABI
Phase 1 MUST record whether each parameter of an exported function is a vararg, with its type recorded as the `List<E>` its body sees, and the ABI hash MUST cover the flag, so an importer packs the elements of its calls and is checked again when the flag changes. `prism abi` MUST print such a parameter as `vararg name: E`.

#### Scenario: Vararg parameters round-trip
- **WHEN** `lib/math.prism` exports `public fun sum(vararg xs: Int): Int = xs.size` and `app/main.prism` imports `/lib/math` and declares `fun f(): Int = sum(1, 2, 3)`
- **THEN** `prism abi lib/math.prism` shows `vararg xs: Int`, and `prism check app/main.prism` passes

#### Scenario: Making a parameter vararg changes the hash
- **WHEN** the exported `public fun sum(xs: List<Int>): Int` of `lib/math` becomes `public fun sum(vararg xs: Int): Int` while an importer calls `sum(listOf(1))`
- **THEN** the file's ABI hash changes, and the importer is checked again and rejected with `E4004`, since `listOf(1)` is not an `Int`
