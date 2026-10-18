# Ownership Specification Delta

## MODIFIED Requirements

### Requirement: Moves Take Whole Values Only
A non-`Copy` value in a consuming position MUST be a whole owned local or by-value parameter, or a temporary (a call, constructor, literal or variant). Taking it from a field (also through a borrow, `Heap<T>` or a temporary), a safe navigation or a top-level `val` MUST be rejected with `CannotMoveOut` (`E4017`), naming the source. Its help MUST suggest a borrow and, for a `Clone` type, `.clone()`, and, for a field of a whole owned local, a destructuring declaration. The error type is never reported.

#### Scenario: Moving a non-Copy field rejected
- **WHEN** `struct User { val id: Int, val name: String }` is declared and a body of `fun f(u: User)` declares `val n = u.name`
- **THEN** the type checker emits `E4017` stating that `name` cannot be moved out of `u`, with help suggesting `&u.name`, `u.name.clone()` or `val (name) = u`, while `val i = u.id` in the same body is accepted

#### Scenario: Help for a field that cannot be cloned
- **WHEN** `struct Token { val id: Int }` and `struct Session { val token: Token }` are declared and `fun f(s: Session): Token = s.token` is checked
- **THEN** the type checker emits `E4017`, with help suggesting `&s.token` and no `.clone()`

#### Scenario: Field moves at every consuming site
- **WHEN** with `fun take(s: String)` and `struct Tag { val s: String }` declared, `u.name` for `u: User` is passed to `take`, returned from a function returning `String`, passed to the constructor `Tag(u.name)`, or assigned to a `var` local
- **THEN** the type checker emits `E4017` at each site, and `u.name.clone()` is accepted at each site

#### Scenario: Moving a field through a borrow or Heap
- **WHEN** `fun f(u: &User): String = u.name`, `fun g(u: &mut User): String = u.name` or `fun h(u: Heap<User>): String = u.name` is checked
- **THEN** the type checker emits `E4017` for each

#### Scenario: Moving a field out of a temporary
- **WHEN** `fun make(): User` is declared and `fun f(): String = make().name` is checked
- **THEN** the type checker emits `E4017`

#### Scenario: Moving through a safe navigation
- **WHEN** `fun f(u: User?): String? = u?.name` is checked
- **THEN** the type checker emits `E4017`, while `fun g(u: User?): Int? = u?.id` and `fun h(u: User?): String? = u?.name?.clone()` typecheck successfully

#### Scenario: Moving out of a top-level val
- **WHEN** a file declares `val GREETING = "hi"` and `val LIMIT = 10` at top level, and `fun f(): String = GREETING` and `fun g(): Int = LIMIT`
- **THEN** the type checker emits `E4017` for `GREETING`, with help suggesting `&GREETING` or `GREETING.clone()`, and accepts `g`

#### Scenario: Whole values and temporaries accepted
- **WHEN** `fun take(u: User)` is declared and a body of `fun f(u: User)` runs `val v = u`, `take(v)`, `take(User(1, "a"))`, `take(make())` and `val h = helper` for a top-level `fun helper(): Int`
- **THEN** the body typechecks successfully; whether `u` or `v` is used after it moved is checked by the borrow checker, not by this rule

#### Scenario: Field of a type from an unimported module
- **WHEN** `shop/models` exports `public struct Money : Copy { val cents: Int }` and `public struct Note : Clone { val text: String }`, `shop/api` exports `public struct Order { val price: Money, val note: Note }`, and a file that imports only `/shop/api` declares `fun f(o: Order) { val p = o.price; val c = o.note.clone(); val n = o.note }`
- **THEN** the type checker accepts `o.price` and `o.note.clone()` and emits `E4017` for `val n = o.note`, without reporting `E4014`

## ADDED Requirements

### Requirement: Taking a Value Apart Moves It Whole
A pattern or destructuring declaration that binds a non-`Copy` part by value MUST move its whole subject. That subject MUST be a whole owned local, a by-value parameter, the `this` of a `consuming fun`, or a call, constructor or literal temporary. A field, a borrow's referent, a top-level `val`, a captured or default-borrowed binding, or a branch, block or `?:` expression MUST be `E4017`, with help suggesting a borrow (`&`).

#### Scenario: Taking a field apart rejected
- **WHEN** `enum Msg { Text(String), Empty }` and `struct Inbox { val last: Msg }` are declared and `fun f(i: Inbox): String = when (i.last) { Msg.Text(val s) -> s; Msg.Empty -> "" }` is checked
- **THEN** the type checker emits `E4017` stating that field `last` of `i` cannot be taken apart, with help suggesting `when (&i.last)`

#### Scenario: Taking a top-level value apart rejected
- **WHEN** with that `Msg`, a file declares `val WELCOME = Msg.Text("hi")` and `fun f(): String = when (WELCOME) { Msg.Text(val s) -> s; Msg.Empty -> "" }`
- **THEN** the type checker emits `E4017` for the top-level value `WELCOME`

#### Scenario: Taking a branch result apart rejected
- **WHEN** with that `Msg`, `fun f(c: Bool, a: Msg, b: Msg): String = when (if (c) a else b) { Msg.Text(val s) -> s; Msg.Empty -> "" }` is checked
- **THEN** the type checker emits `E4017`, with help suggesting binding the value to a `val` first

#### Scenario: Owned locals and temporaries are taken apart
- **WHEN** with that `Msg`, `fun make(): Msg` is declared and `fun f(m: Msg): String = when (m) { Msg.Text(val s) -> s; Msg.Empty -> "" }` and `fun g(): String = when (make()) { Msg.Text(val s) -> s; Msg.Empty -> "" }` are checked
- **THEN** both typecheck successfully

#### Scenario: Copy parts and wildcards do not take a value apart
- **WHEN** `enum Pair { Of(String, Int) }` and `struct Inbox { val last: Pair }` are declared and `fun f(i: Inbox): Int = when (i.last) { Pair.Of(_, val n) -> n }` is checked
- **THEN** the file typechecks, since `n` is copied and the field is only read

### Requirement: Use After Taking Apart Is Rejected
A value that a branch or destructuring declaration took apart MUST be moved on that path, so a later use of it MUST be `E5001`, whose "value moved here" label points at the binding that moved a part. When the value was a `when` subject, the help MUST suggest matching on its borrow (`when (&m)`).

#### Scenario: Using a subject after a branch took it apart
- **WHEN** `enum Msg { Text(String), Empty }` and `fun show(m: &Msg) {}` are declared and `fun f(m: Msg) { when (m) { Msg.Text(val s) -> println(s); Msg.Empty -> {} }; show(m) }` is checked
- **THEN** the borrow checker emits `E5001` at `m` in `show(m)`, labelling `val s` as where the value moved, with help suggesting `when (&m)`

#### Scenario: A branch that does not take the subject apart leaves it usable
- **WHEN** with that `Msg` and `show`, `fun f(m: Msg) { when (m) { Msg.Text(_) -> println(1); Msg.Empty -> {} }; show(m) }` is checked
- **THEN** the file passes the borrow checker

#### Scenario: Using a destructured value
- **WHEN** `struct User { val name: String, val id: Int }` and `fun show(u: &User) {}` are declared and `fun f(u: User) { val (name) = u; show(u) }` is checked
- **THEN** the borrow checker emits `E5001` at `u` in `show(u)`, labelling the entry `name` as where the value moved
