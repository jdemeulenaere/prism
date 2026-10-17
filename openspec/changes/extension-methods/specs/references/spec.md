# References Specification Delta

## ADDED Requirements

### Requirement: Method Receivers Borrow Implicitly
The receiver of an extension call MUST be passed by the extension's receiver mode with nothing written at the call site: borrowed shared for `fun`, borrowed mutably for `mut fun`, and by value for `consuming fun` (see `ownership`). Any number of `&`, `&mut` and `Heap<T>` layers around the receiver MUST be looked through. A place MUST be borrowed where it lives, and any other receiver MUST be a temporary, in a consuming position, that lives until the end of the statement.

#### Scenario: Shared receivers of places, references and temporaries
- **WHEN** `struct User { val name: String }`, `fun User.greet(): String`, `fun String.shout(): String` and `fun make(): User` are declared, and a body of `fun f(u: User, r: &User, rr: &&User, m: &mut User, h: Heap<User>)` runs `u.greet()`, `u.name.shout()`, `r.greet()`, `rr.greet()`, `m.greet()`, `h.greet()`, `make().greet()` and `"a".shout()`
- **THEN** the body typechecks successfully, and neither `u` nor `u.name` is reported as moved

#### Scenario: A branch receiver is a temporary
- **WHEN** `fun String.shout(): String` and `struct User { val name: String }` are declared and `fun f(c: Bool, u: User, v: User): String = (if (c) u.name else v.name).shout()` is checked
- **THEN** the type checker emits `E4017` for `u.name` and for `v.name`

### Requirement: Mutable Receivers Must Be Mutable Places
The receiver of a `mut fun` call MUST be borrowed mutably without `&mut`. It MUST be a `var` binding, a field path whose every hop is mutable, a place reached through `&mut` (a `val` binding holding a `&mut T` is reborrowed), or a temporary. A `val` binding, a `val` field, or a place reached through a shared `&` MUST be rejected with `E4003`.

#### Scenario: Method receivers borrow implicitly
- **WHEN** `struct User { var name: String }` and `mut fun User.rename(n: String) { name = n }` are declared, and a body declares `var u = User("a")` and `val w = User("b")` and calls `u.rename("x")` and `w.rename("y")`
- **THEN** the type checker accepts `u.rename("x")`, which borrows `u` mutably without `&mut`, and emits `E4003` for `w.rename("y")`, with help suggesting `var w`

#### Scenario: Mutable receivers through references
- **WHEN** `mut fun User.rename(n: String)` is declared, and `fun f(m: &mut User) { m.rename("x"); m.rename("y") }` and `fun g(r: &User) { r.rename("x") }` are checked
- **THEN** the type checker accepts `f`, reborrowing `m` for each call, and emits `E4003` for `g`

#### Scenario: Mutable receiver on a field path
- **WHEN** `struct User { var name: String }`, `struct Team { var lead: User, val owner: User }` and `mut fun User.rename(n: String)` are declared, and a body declares `var t: Team` and calls `t.lead.rename("x")` and `t.owner.rename("y")`, and `fun h(p: &mut Team) { p.lead.rename("z") }` is checked
- **THEN** the type checker accepts `t.lead.rename("x")` and `h`, and emits `E4003` for `t.owner.rename("y")`

#### Scenario: Mutable receiver of a temporary
- **WHEN** `fun make(): User` and `mut fun User.rename(n: String)` are declared and `fun f() { make().rename("x") }` is checked
- **THEN** the file typechecks successfully, as `&mut make()` does

#### Scenario: Mutable call on the implicit receiver
- **WHEN** `mut fun User.rename(n: String)` is declared, and `mut fun User.reset() { rename("") }` and `fun User.peek() { rename("") }` are checked
- **THEN** the type checker accepts `reset`, reborrowing `this`, and emits `E4003` for `peek`, because its `this` is a shared borrow

#### Scenario: Mutable safe call on a nullable place
- **WHEN** `mut fun User.rename(n: String)` is declared, and a body declares `var u: User? = null` and `val v: User? = null` and runs `u?.rename("x")` and `v?.rename("y")`
- **THEN** the type checker accepts `u?.rename("x")` and emits `E4003` for `v?.rename("y")`

## MODIFIED Requirements

### Requirement: Explicit Mutable Borrows at Call Sites
A parameter of type `&mut T` MUST NOT borrow an owned argument implicitly. The caller MUST write `&mut`, and an owned place passed without it MUST be rejected with `E4004`, with help suggesting `&mut` before the argument. The receiver of a `mut fun` call is the one exception: it is borrowed mutably without `&mut` (see "Mutable Receivers Must Be Mutable Places").

#### Scenario: Explicit mutation at call sites
- **WHEN** `fun bump(r: &mut Int)` is declared, and a body declares `var x = 1` and calls `bump(x)` and `bump(&mut x)`
- **THEN** the type checker emits `E4004` for `bump(x)`, with help suggesting `&mut x`, and accepts `bump(&mut x)`
