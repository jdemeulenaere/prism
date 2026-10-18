# Ownership Specification Delta

## ADDED Requirements

### Requirement: Unboxing Moves the Contents Out
A `Heap<T>`, after its references are peeled, MUST have a built-in member function `unbox()` of type `T`, taking no arguments (`E4004` otherwise), whose receiver is consuming: it MUST move the contents out of the box and free the box. Its receiver MUST follow the move rules of consuming receivers. `h?.unbox()` on an owned `Heap<T>?` MUST have type `T?`.

#### Scenario: Unboxing a whole value
- **WHEN** `struct User { val name: String }` is declared and `fun f(h: Heap<User>): User = h.unbox()`, `fun g(): Int = Heap(1).unbox()` and `fun k(hh: Heap<Heap<Int>>): Heap<Int> = hh.unbox()` are checked
- **THEN** all three typecheck; a later use of `h` in `f` would be a use after move, which the borrow checker reports

#### Scenario: Unboxing a field or through a borrow rejected
- **WHEN** `struct Node : Clone { val value: Int, val next: Heap<Node>? }` and `struct Holder { val h: Heap<Int> }` are declared, and `fun f(o: Holder): Int = o.h.unbox()` and `fun g(r: &Heap<Node>): Node = r.unbox()` are checked
- **THEN** the type checker emits `E4017` for the field `h` of `o`, and for `g`, naming the borrow `r`, with help suggesting `r.clone().unbox()`

#### Scenario: Unboxing an optional box
- **WHEN** `struct Node { val value: Int, val next: Heap<Node>? }` is declared and `fun f(n: Heap<Node>?): Node? = n?.unbox()` is checked
- **THEN** the file typechecks successfully

#### Scenario: unbox takes no arguments
- **WHEN** `fun f(h: Heap<Int>): Int = h.unbox(1)` is checked
- **THEN** the type checker emits `E4004`, saying that `unbox()` takes no arguments

### Requirement: Moving Out of a Box Is Rejected
Where a value of type `T` is expected and an expression of type `Heap<T>`, behind any boxes and references, is given for a non-`Copy` `T`, the type checker MUST reject it with `CannotMoveOut` (`E4017`), naming the source "the contents of `Heap` `h`", with help suggesting `h.unbox()`. A `Copy` value is copied out instead (see the `references` spec).

#### Scenario: Taking a box's contents where a value is expected
- **WHEN** `struct User { val name: String }` and `fun take(u: User) {}` are declared, and `fun f(h: Heap<User>) { take(h) }`, `fun g(h: Heap<User>): User = h` and `fun k(h: Heap<User>) { val u: User = h }` are checked
- **THEN** the type checker emits `E4017` at each, naming the contents of `Heap` `h`, with help suggesting `h.unbox()`

## MODIFIED Requirements

### Requirement: Consuming Receivers Follow the Move Rules
The receiver of a `consuming fun` call MUST be checked as a consuming position. A whole local, parameter or temporary MUST be moved into the call, and when it is reached through `Heap<T>` layers only, each box MUST be unboxed: its contents moved out and the box freed. A field, a top-level `val`, an index or a safe navigation of a field MUST be rejected with `E4017`, as for any move. Through a reference, a `Copy` receiver MUST be copied out and any other MUST be rejected with `E4017`.

#### Scenario: Consuming method moves the receiver
- **WHEN** `struct User { val name: String }`, `struct Account { val owner: User }`, `consuming fun User.toAccount(): Account = Account(this)` and `fun make(): User` are declared and a body of `fun f(u: User)` runs `val a = u.toAccount()` and `val b = make().toAccount()`
- **THEN** the body typechecks successfully; a later use of `u` is a use after move, which the borrow checker reports, not the type checker

#### Scenario: Consuming a field receiver rejected
- **WHEN** `struct User : Clone { val name: String }`, `struct Order { val user: User }` and `consuming fun User.toAccount(): Account` are declared and `fun f(o: Order): Account = o.user.toAccount()` is checked
- **THEN** the type checker emits `E4017` for the field `user` of `o`, with help suggesting `o.user.clone()`

#### Scenario: Consuming through a borrow
- **WHEN** `consuming fun User.toAccount(): Account`, `struct Point : Copy { val x: Int }` and `consuming fun Point.moved(): Point` are declared, and `fun f(r: &User): Account = r.toAccount()`, `fun g(h: &Heap<User>): Account = h.toAccount()` and `fun k(p: &Point): Point = p.moved()` are checked
- **THEN** the type checker emits `E4017` for `f`, naming the borrow `r`, and for `g`, naming the borrow `h`, and accepts `k`, copying the `Point` out

#### Scenario: Consuming through a box
- **WHEN** `consuming fun User.toAccount(): Account`, `struct Team { val lead: Heap<User> }` are declared, and `fun f(h: Heap<User>): Account = h.toAccount()`, `fun g(hh: Heap<Heap<User>>): Account = hh.toAccount()` and `fun k(t: Team): Account = t.lead.toAccount()` are checked
- **THEN** the type checker accepts `f` and `g`, each unboxing its receiver, and emits `E4017` for `k`, because `t.lead` is a field

#### Scenario: Consuming a top-level val receiver rejected
- **WHEN** a file declares `val GUEST = User("guest")` and `consuming fun User.toAccount(): Account`, and `fun f(): Account = GUEST.toAccount()`
- **THEN** the type checker emits `E4017` for the top-level value `GUEST`

#### Scenario: Consuming safe call
- **WHEN** `consuming fun User.toAccount(): Account` and `struct Team { val lead: User? }` are declared, and `fun f(u: User?): Account? = u?.toAccount()` and `fun g(t: Team): Account? = t.lead?.toAccount()` are checked
- **THEN** the type checker accepts `f`, which moves `u`, and emits `E4017` for `g`, because `t.lead` is a field
