## ADDED Requirements

### Requirement: Field Visibility in the ABI
Every field of an exported struct, hidden or not, MUST be in its ABI entry with its visibility (`private`, `package` or unmarked, an explicit `public` recorded as unmarked), and the ABI hash MUST cover it. `prism abi` MUST print a marked field with its modifier (`private val id: Int`). Changing only a field's visibility MUST change the ABI hash, so importers are checked again, and MUST change no layout, slot contract or type fingerprint.

#### Scenario: Hiding a field re-checks importers
- **WHEN** `lib/id.prism` exports `public struct Id : Copy { val n: Int }` and `public fun id(n: Int): Id = Id(n)`, `app/main.prism` imports `/lib/id` and declares `fun f(x: Id): Int = x.n`, both have been checked, and `n` becomes `private val n: Int`
- **THEN** the next `prism check` re-checks `app/main` and reports `E4007` at `n`, although `app/main` itself was not edited

#### Scenario: The ABI shows field modifiers
- **WHEN** `prism abi` prints the ABI of a file declaring `public struct Account { private var balance: Int, package val bank: String, public val owner: String }`
- **THEN** it shows the fields `private var balance: Int`, `package val bank: String` and `val owner: String`
