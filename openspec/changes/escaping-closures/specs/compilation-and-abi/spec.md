# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Escaping Parameters in the ABI
Phase 1 MUST record whether each parameter of an exported function, and each parameter of every function type in an exported signature, is `escaping`, and the ABI and interface hashes MUST cover it, so an importer obtains the flags the exporter declared and is re-checked when one changes.

#### Scenario: Escaping parameters round-trip
- **WHEN** `lib/ui.prism` exports `public fun button(onClick: escaping () -> Unit, register: (escaping () -> Unit) -> Unit): Int` and `app/main.prism` imports `/lib/ui` and declares `fun greet(s: &String) {}` and `fun f(s: String): Int = button({ greet(s) }) { h -> h() }`
- **THEN** `prism abi lib/ui.prism` shows `onClick: escaping () -> Unit` and `register: (escaping () -> Unit) -> Unit`, and `prism check app/main.prism` passes, the first closure owning `s`

#### Scenario: Marking a parameter escaping changes the hashes
- **WHEN** an exported `public fun store(f: () -> Unit): Int` becomes `public fun store(f: escaping () -> Unit): Int`
- **THEN** the file's ABI hash and interface hash both change, and an importing file is checked again
