## ADDED Requirements

### Requirement: Only Visible Fields Count
Wherever a field takes part in resolving a name (a call `value.name(...)`, a property read, a bare name in an extension body), only a field visible in that file MUST count. An extension function or extension property named like a field of its receiver struct MUST be `E4010` only when that field is visible in the extension's file. A hidden field MUST NOT shadow a member, extension or outer binding of its name.

#### Scenario: An extension named like a hidden field
- **WHEN** `bank/account.prism` exports `public struct Account { private val id: Int, val owner: String }` and `public fun open(id: Int): Account = Account(id, "ann")`, and a file importing `/bank/account` and `/prism/core` declares `fun Account.id(): Int = 42` and `fun main() { println(open(7).id()) }`, and runs with `--check-allocations`
- **THEN** the file typechecks and stdout is `42`: the private field `id` is not visible there, so the call resolves to the file's own extension

#### Scenario: An extension named like a visible field is still rejected
- **WHEN** a file declares `struct Account { private val id: Int }` and `fun Account.id(): Int = 1`
- **THEN** the type checker emits `E4010` for the extension, saying that the field `id` always takes precedence in this file

#### Scenario: A hidden field does not shadow an outer name
- **WHEN** `lib/box.prism` exports `public struct Box : Copy { private val size: Int }` and `public fun box(): Box = Box(3)`, and a file importing `/lib/box` declares `val size: Int = 10` and `fun Box.twice(): Int = size * 2`
- **THEN** the file typechecks, and `size` in `twice` reads the file's top-level value
