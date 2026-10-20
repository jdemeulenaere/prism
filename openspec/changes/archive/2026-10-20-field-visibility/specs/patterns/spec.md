## ADDED Requirements

### Requirement: Patterns Name Only Visible Fields
A struct pattern or destructuring declaration MUST be rejected with `E4007` where it gives a field not visible in the file: a positional pattern gives every field, a positional sub-pattern before named ones the field it stands for, and a named sub-pattern or destructuring entry the field it names. Patterns and declarations that leave hidden fields out MUST stay valid, the left-out fields matched by wildcards, and taking a value apart through them MUST move only the parts they name.

#### Scenario: Patterns over a struct with a private field
- **WHEN** `bank/account.prism` exports `public struct Account { private val id: Int, val owner: String }` and `public fun open(owner: String): Account = Account(1, owner)`, and a file importing `/bank/account` declares `fun a(x: &Account): Int = when (x) { Account(_, _) -> 1 }`, `fun b(x: &Account): Int = when (x) { Account(id = 1) -> 1; else -> 2 }`, `fun c(x: &Account): Bool = when (x) { Account(owner = "ann") -> true; else -> false }` and `fun d(x: Account): String { val (owner) = x; return owner }`
- **THEN** the type checker emits `E4007` in `a` (a positional pattern of a struct with a hidden field) and in `b` (the private field `id`), and accepts `c` and `d`

#### Scenario: Destructuring a hidden field rejected
- **WHEN** with that `Account`, a file importing `/bank/account` declares `fun e(x: &Account): Int { val (id) = x; return id }`
- **THEN** the type checker emits `E4007` at `id` in the destructuring declaration

#### Scenario: Positional sub-patterns before named ones
- **WHEN** `bank/account.prism` exports `public struct Account { private val id: Int, val owner: String, val bank: String }`, and a file importing `/bank/account` declares `fun f(x: &Account): Int = when (x) { Account(val i, bank = _) -> i }` and `fun g(x: &Account): Int = when (x) { Account(_, owner = "ann") -> 1; else -> 2 }`
- **THEN** the type checker emits `E4007` at the first sub-pattern of each, which stands for the private field `id`; a positional prefix that stands for visible fields only, as `Entry(val o, note = "paid")` of `public struct Entry { val owner: String, private val id: Int, val note: String }`, stays valid
