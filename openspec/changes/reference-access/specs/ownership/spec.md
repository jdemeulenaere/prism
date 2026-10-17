# Ownership Specification Delta

## ADDED Requirements

### Requirement: Moving Out of a Borrow Is Rejected
Where a value of type `T` is expected and an expression of type `&T` or `&mut T` is given for a non-`Copy` `T`, the type checker MUST reject it with `CannotMoveOut` (`E4017`), naming the source "borrow `u`", where `u` is the expression. Its help MUST suggest `u.clone()` when `T` is `Clone`, and otherwise suggest taking a `&T`. A `Copy` value is copied out instead (see the `references` spec).

#### Scenario: Move out of a borrow rejected
- **WHEN** `struct User : Clone { val name: String }` and `fun take(u: User)` are declared and `fun f(u: &User) { take(u) }` is checked
- **THEN** the type checker rejects the argument as a move out of a borrow: `E4017` with the source "borrow `u`", with help suggesting `u.clone()`

#### Scenario: Help for a borrow that cannot be cloned
- **WHEN** `struct Token { val id: Int }` is declared and `fun f(t: &Token): Token = t` is checked
- **THEN** the type checker emits `E4017` with the source "borrow `t`", with help suggesting a `&Token` and no `.clone()`

#### Scenario: Moves out of a borrow at every consuming site
- **WHEN** `struct Tag { val s: String }` and `fun get(s: &String): &String` are declared and a body of `fun f(r: &String, m: &mut String)` runs `val a: String = r`, `var b = "x"`, `b = m`, `val t = Tag(r)` and `val c: String = get(r)`
- **THEN** the type checker emits `E4017` at each of the four sites that take a borrow, naming the borrow `r`, `m`, `r` and `get(r)`, with help suggesting `.clone()`

## MODIFIED Requirements

### Requirement: Consuming Positions
A value MUST be consumed when it initializes a local, a parameter default or a top-level `val`; is assigned, also through `*`; is passed to a by-value parameter, struct field or enum payload; or is returned (a `return` operand, an expression body, a block tail or closure result). A branch, block tail, parenthesized expression or `?:` operand in a consuming position is consuming. A borrowed operand that is not a place is consumed (see `references`). Every other use reads in place.

#### Scenario: Branch results in a consuming position
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: User, c: Bool): String { val n = if (c) u.name else "x"; return n }` is checked
- **THEN** the type checker emits `E4017` for `u.name`

#### Scenario: Elvis operands in a consuming position
- **WHEN** `struct User { val nick: String? }` is declared and `fun f(u: User): String = u.nick ?: "anonymous"` is checked
- **THEN** the type checker emits `E4017` for `u.nick`, with help suggesting `u.nick?.clone()`

#### Scenario: Reads in place are not moves
- **WHEN** `struct User { val name: String }` is declared and a body of `fun f(u: User): Bool` uses `u.name` as an operand (`u.name == "a"`, `u.name + "!"`), a `when` subject, the operand of `&` (`val r = &u.name`), the receiver of `clone()` or a discarded expression statement
- **THEN** the body typechecks successfully

#### Scenario: Borrowing a branch or elvis result
- **WHEN** `struct User { val name: String, val nick: String? }` is declared and a body of `fun f(c: Bool, u: User, v: User)` runs `val r = &(if (c) u.name else v.name)` and `val s = &(u.nick ?: "x")`
- **THEN** the type checker emits `E4017` for `u.name`, `v.name` and `u.nick`, because each borrow's operand is a temporary made by moving them

#### Scenario: Borrowing a safe navigation
- **WHEN** `struct User { val name: String, val id: Int }` is declared and a body of `fun f(u: User?)` runs `val r = &u?.name` and `val i = &u?.id`
- **THEN** the type checker emits `E4017` for `u?.name`, with help suggesting `u?.name?.clone()`, and accepts `&u?.id`, whose temporary is a copy
