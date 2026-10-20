## ADDED Requirements

### Requirement: A Field Is Visible Where Its Modifier Allows
A field marked `private` MUST be visible only in its struct's file, a field marked `package` only in files of its struct's package directory, and a field marked `public` or unmarked wherever its struct is visible. A field MUST never be more visible than its struct.

#### Scenario: Unmarked fields keep their meaning
- **WHEN** `shop/models.prism` exports `public struct Order { val price: Int }` and a file importing `/shop/models` declares `fun f(o: &Order): Int = o.price` and `fun g(): Order = Order(3)`
- **THEN** the file typechecks, as before field visibility

#### Scenario: A package field is visible in its package
- **WHEN** `shop/models.prism` declares `package shop` and `public struct Order { package val cost: Int, val price: Int }`, `shop/report.prism` in the same package declares `fun margin(o: &Order): Int = o.price - o.cost`, and `app/main.prism` declares `fun cost(o: &Order): Int = o.cost`, each importing `/shop/models`
- **THEN** `shop/report.prism` typechecks, and `app/main.prism` is rejected with `E4007` at `cost`

### Requirement: Hidden Fields Are Not Accessed
Where a field is not visible, reading, writing or borrowing it, capturing its path, or reading, writing or borrowing it through a bare name in an extension body MUST be `E4007`, through any `&`, `&mut`, `Heap<T>` or `?.`, naming the field, its visibility and its struct's module. When nothing else resolves the name, a hidden field MUST be reported with `E4007`, not `E4006`. Copying, moving, cloning, dropping and comparing whole values MUST be unaffected.

#### Scenario: Reading and writing a private field from another file
- **WHEN** `bank/account.prism` exports `public struct Account { private var balance: Int, val owner: String }`, `public fun open(owner: String): Account = Account(0, owner)` and `public mut fun Account.deposit(n: Int): Unit { balance += n }`, and a file importing `/bank/account` declares `fun f(a: &mut Account): Int { a.balance = 5; return a.balance }` and `fun g(a: &Account): Int = a.owner.length`
- **THEN** the type checker emits `E4007` at each `balance` in `f`, naming the private field `balance` of `bank/account`, and accepts `g`

#### Scenario: A private field in its own file
- **WHEN** a file declares `struct Account { private var balance: Int }`, `mut fun Account.deposit(n: Int) { balance += n }` and `fun main() { var a = Account(1); a.deposit(2); println(a.balance) }`, and runs with `--check-allocations`
- **THEN** stdout is `3`, and the exit status is 0

#### Scenario: Whole values with hidden fields are used freely
- **WHEN** `lib/id.prism` exports `public struct Id : Copy { private val n: Int }` and `public fun id(n: Int): Id = Id(n)`, and a file importing `/lib/id` and `/prism/core` runs `fun main() { val a = id(1); val b = a; println(a == b); println(a == id(2)) }` with `--check-allocations`
- **THEN** stdout is `true` then `false`, and the exit status is 0

### Requirement: Constructing a Struct Needs Every Field Visible
A construction of a struct, positional or named, MUST be rejected with `E4007` where any of its fields is not visible, naming the first such field. A struct with a `private` field MUST therefore be constructed only in its own file.

#### Scenario: A handle cannot be forged
- **WHEN** `lib/handle.prism` exports `public struct Handle : Copy { private val word: Int }` and `public fun make(): Handle = Handle(7)`, and a file importing `/lib/handle` declares `fun forge(): Handle = Handle(7)` and `fun named(): Handle = Handle(word = 7)`
- **THEN** `lib/handle.prism` typechecks, and the type checker emits `E4007` for each construction in the importing file, naming the private field `word`
