# Ownership Specification Delta

## ADDED Requirements

### Requirement: Consuming Receivers Follow the Move Rules
The receiver of a `consuming fun` call MUST be checked as a consuming position. A whole local, parameter or temporary MUST be moved into the call. A field, a top-level `val`, an index or a safe navigation of a field MUST be rejected with `E4017`, as for any move. Through a reference or `Heap<T>`, a `Copy` receiver MUST be copied out and any other MUST be rejected with `E4017`.

#### Scenario: Consuming method moves the receiver
- **WHEN** `struct User { val name: String }`, `struct Account { val owner: User }`, `consuming fun User.toAccount(): Account = Account(this)` and `fun make(): User` are declared and a body of `fun f(u: User)` runs `val a = u.toAccount()` and `val b = make().toAccount()`
- **THEN** the body typechecks successfully; a later use of `u` is a use after move, which the borrow checker reports, not the type checker

#### Scenario: Consuming a field receiver rejected
- **WHEN** `struct User : Clone { val name: String }`, `struct Order { val user: User }` and `consuming fun User.toAccount(): Account` are declared and `fun f(o: Order): Account = o.user.toAccount()` is checked
- **THEN** the type checker emits `E4017` for the field `user` of `o`, with help suggesting `o.user.clone()`

#### Scenario: Consuming through a borrow
- **WHEN** `consuming fun User.toAccount(): Account`, `struct Point : Copy { val x: Int }` and `consuming fun Point.moved(): Point` are declared, and `fun f(r: &User): Account = r.toAccount()`, `fun g(h: Heap<User>): Account = h.toAccount()` and `fun k(p: &Point): Point = p.moved()` are checked
- **THEN** the type checker emits `E4017` for `f`, naming the borrow `r`, and for `g`, and accepts `k`, copying the `Point` out

#### Scenario: Consuming a top-level val receiver rejected
- **WHEN** a file declares `val GUEST = User("guest")` and `consuming fun User.toAccount(): Account`, and `fun f(): Account = GUEST.toAccount()`
- **THEN** the type checker emits `E4017` for the top-level value `GUEST`

#### Scenario: Consuming safe call
- **WHEN** `consuming fun User.toAccount(): Account` and `struct Team { val lead: User? }` are declared, and `fun f(u: User?): Account? = u?.toAccount()` and `fun g(t: Team): Account? = t.lead?.toAccount()` are checked
- **THEN** the type checker accepts `f`, which moves `u`, and emits `E4017` for `g`, because `t.lead` is a field

## MODIFIED Requirements

### Requirement: Consuming Positions
A value MUST be consumed when it initializes a local, a parameter default or a top-level `val`; is assigned, also through `*`; is passed to a by-value parameter, struct field or enum payload; is the receiver of a `consuming fun` call; or is returned (a `return` operand, an expression body, a block tail or closure result). A branch, block tail, parenthesized expression or `?:` operand in a consuming position is consuming. A borrowed operand or receiver that is not a place is consumed (see `references`). Every other use reads in place.

#### Scenario: Branch results in a consuming position
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: User, c: Bool): String { val n = if (c) u.name else "x"; return n }` is checked
- **THEN** the type checker emits `E4017` for `u.name`

#### Scenario: Elvis operands in a consuming position
- **WHEN** `struct User { val nick: String? }` is declared and `fun f(u: User): String = u.nick ?: "anonymous"` is checked
- **THEN** the type checker emits `E4017` for `u.nick`, with help suggesting `u.nick?.clone()`

#### Scenario: Reads in place are not moves
- **WHEN** `struct User { val name: String }` and `fun String.shout(): String` are declared and a body of `fun f(u: User): Bool` uses `u.name` as an operand (`u.name == "a"`, `u.name + "!"`), a `when` subject, the operand of `&` (`val r = &u.name`), the receiver of `clone()`, the receiver of a plain or `mut fun` extension call (`u.name.shout()`) or a discarded expression statement
- **THEN** the body typechecks successfully

#### Scenario: Borrowing a branch or elvis result
- **WHEN** `struct User { val name: String, val nick: String? }` is declared and a body of `fun f(c: Bool, u: User, v: User)` runs `val r = &(if (c) u.name else v.name)` and `val s = &(u.nick ?: "x")`
- **THEN** the type checker emits `E4017` for `u.name`, `v.name` and `u.nick`, because each borrow's operand is a temporary made by moving them

#### Scenario: Borrowing a safe navigation
- **WHEN** `struct User { val name: String, val id: Int }` is declared and a body of `fun f(u: User?)` runs `val r = &u?.name` and `val i = &u?.id`
- **THEN** the type checker emits `E4017` for `u?.name`, with help suggesting `u?.name?.clone()`, and accepts `&u?.id`, whose temporary is a copy

### Requirement: Explicit Clone
Every `Clone` type MUST have a built-in member `clone()` taking no arguments, unless the type has a field named `clone`, which takes precedence. It reads its receiver in place, looking through `&` and `&mut`, and returns an owned temporary of the receiver's type with those borrows removed. `x?.clone()` MUST work on nullables. On a type that is not `Clone`, `x.clone()` MUST call an extension named `clone` when one is visible (see `extension-functions`), and otherwise be rejected with `NotCloneable` (`E4018`) naming the type.

#### Scenario: Cloning a field through a borrow
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: &User): String = u.name.clone()` is checked
- **THEN** the file typechecks successfully

#### Scenario: Cloning with safe navigation and elvis
- **WHEN** `struct User { val nick: String? }` is declared and `fun f(u: &User): String = u.nick?.clone() ?: "anonymous"` is checked
- **THEN** the file typechecks successfully

#### Scenario: Cloning a Heap
- **WHEN** `struct User : Clone { val name: String }` is declared and `fun f(h: &Heap<User>): Heap<User> = h.clone()` is checked
- **THEN** the file typechecks successfully

#### Scenario: Cloning a type without the marker rejected
- **WHEN** `struct Token { val id: Int }` is declared and `fun f(t: &Token): Token = t.clone()` is checked
- **THEN** the type checker emits `E4018` naming `Token`, with help suggesting `: Clone` on its declaration

#### Scenario: Cloning a function value rejected
- **WHEN** `fun f(g: () -> Unit) { val h = g.clone() }` is checked
- **THEN** the type checker emits `E4018`

#### Scenario: Clone takes no arguments
- **WHEN** `fun f(s: &String): String = s.clone(1)` is checked
- **THEN** the type checker emits `E4004`

#### Scenario: A field named clone wins over the built-in
- **WHEN** `struct S { val clone: () -> Int }` is declared and `fun f(s: &S): Int = s.clone()` is checked
- **THEN** the file typechecks successfully: the call invokes the field, because a member of the receiver's type takes precedence over the built-in `clone()`

#### Scenario: An extension named clone on a type that is not Clone
- **WHEN** `struct Token { val id: Int }` and `fun Token.clone(): Token = Token(id)` are declared and `fun f(t: &Token): Token = t.clone()` is checked
- **THEN** the file typechecks successfully: the call invokes the extension instead of reporting `E4018`
