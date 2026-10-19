## ADDED Requirements

### Requirement: Attached Parameters and Capture Functions in the ABI
Phase 1 MUST record whether each parameter of an exported function, and each parameter of every function type in an exported signature, is `attached`, and whether an exported intrinsic function is a capture function, and the ABI hash MUST cover both, so an importer obtains what the exporter declared and is checked again when one changes.

#### Scenario: Attached parameters round-trip
- **WHEN** `lib/ui.prism` exports `public fun button(onClick: attached () -> Unit): Int` and `app/main.prism` imports `/lib/ui` and declares `fun f(count: &MutableState<Int>): Int = button { count.value += 1 }`
- **THEN** `prism abi lib/ui.prism` shows `onClick: attached () -> Unit`, and `prism check app/main.prism` passes, the closure capturing `count` through a weak handle

#### Scenario: Marking a parameter attached changes the hashes
- **WHEN** an exported `public fun store(f: escaping () -> Unit): Int` becomes `public fun store(f: attached () -> Unit): Int`
- **THEN** the file's ABI hash changes, and an importing file is checked again
