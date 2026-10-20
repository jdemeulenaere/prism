## ADDED Requirements

### Requirement: Patterns Name Only Visible Fields
A positional struct pattern MUST be rejected with `E4007` where any field of its struct is not visible, since it gives every field. A named sub-pattern or a destructuring entry that names a field not visible in the file MUST be rejected with `E4007`. Named patterns and destructuring declarations that leave hidden fields out MUST stay valid, the fields they leave out being matched by wildcards, and taking a value apart through them MUST move only the parts they name.

#### Scenario: Patterns over a struct with a private field
- **WHEN** `bank/account.prism` exports `public struct Account { private val id: Int, val owner: String }` and `public fun open(owner: String): Account = Account(1, owner)`, and a file importing `/bank/account` declares `fun a(x: &Account): Int = when (x) { Account(_, _) -> 1 }`, `fun b(x: &Account): Int = when (x) { Account(id = 1) -> 1; else -> 2 }`, `fun c(x: &Account): Bool = when (x) { Account(owner = "ann") -> true; else -> false }` and `fun d(x: Account): String { val (owner) = x; return owner }`
- **THEN** the type checker emits `E4007` in `a` (a positional pattern of a struct with a hidden field) and in `b` (the private field `id`), and accepts `c` and `d`

#### Scenario: Destructuring a hidden field rejected
- **WHEN** with that `Account`, a file importing `/bank/account` declares `fun e(x: &Account): Int { val (id) = x; return id }`
- **THEN** the type checker emits `E4007` at `id` in the destructuring declaration
