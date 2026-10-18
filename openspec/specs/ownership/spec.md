# Ownership Specification

## Purpose
The ownership specification defines Prism's single-owner value model as the type checker enforces it: which types are `Copy` or `Clone`, how `: Copy` and `: Clone` declarations are checked, explicit cloning, what a move may take its value from, and why top-level declarations are immutable. It also holds the flow-sensitive rules that the borrow checker enforces on MIR: no use of a moved or uninitialized value, and reinitialization by assignment. Borrow conflicts are the `references` spec's.

## Requirements

### Requirement: Copy and Clone Are Declared on Structs and Enums
A struct or enum MUST be `Copy` only when it declares `: Copy`, and `Clone` only when it declares `: Clone` or `: Copy`. Without a marker it MUST be move-only and not cloneable, whatever its fields. A `Copy` value is duplicated implicitly by a bitwise copy that runs no user code. A `Clone` value is duplicated only by an explicit `.clone()`. Every other use of a non-`Copy` value moves it.

#### Scenario: Opt-in Copy struct
- **WHEN** `struct Point : Copy { val x: Double, val y: Double }` and `struct Line { val start: Point, val end: Point }` are declared and `fun f(l: Line): Point = l.start` is checked
- **THEN** the file typechecks successfully, because reading a `Copy` field copies it

#### Scenario: Struct without a marker is move-only
- **WHEN** `struct Point { val x: Int, val y: Int }` (no marker, all fields `Copy`) and `struct Line { val start: Point }` are declared and `fun f(l: Line): Point = l.start` is checked
- **THEN** the type checker emits `E4017`, because `Point` is not `Copy`

#### Scenario: Copy enum with payloads
- **WHEN** `enum Shape : Copy { Circle(Float), Square(Float), Empty }` and `struct Slot { val shape: Shape }` are declared and `fun f(s: Slot): Shape = s.shape` is checked
- **THEN** the file typechecks successfully

#### Scenario: Clone struct is not Copy
- **WHEN** `struct User : Clone { val name: String }` and `struct Team { val lead: User }` are declared, and `fun f(t: &Team): User = t.lead.clone()` and `fun g(t: &Team): User = t.lead` are checked
- **THEN** `f` typechecks successfully and the type checker emits `E4017` for `g`

#### Scenario: Copy implies Clone
- **WHEN** `struct Point : Copy { val x: Int }` is declared and `fun f(p: &Point): Point = p.clone()` is checked
- **THEN** the file typechecks successfully

### Requirement: Built-In Copy Types
`Int`, `Float`, `Bool`, `Char`, `Unit`, `Nothing`, the built-in `Range` and shared borrows `&T` MUST be `Copy`, and `T?` MUST be `Copy` exactly when `T` is. A type parameter MUST be `Copy` exactly when it is bounded by `Copy`. `&mut T`, `String`, `Heap<T>`, function types (composable or not), unbounded or `Clone`-bounded type parameters and type names with no declaration in scope MUST NOT be `Copy`.

#### Scenario: Copy built-ins as fields of a Copy struct
- **WHEN** `struct S : Copy { val a: Int, val b: Float, val c: Bool, val d: Char, val e: Unit, val r: Range, val n: Int? }` is declared
- **THEN** the declaration typechecks successfully

#### Scenario: String is move-only
- **WHEN** `struct User : Copy { val name: String }` is declared
- **THEN** the type checker emits `E4016` naming the field `name` and the type `String`

#### Scenario: Non-Copy built-ins rejected in a Copy declaration
- **WHEN** a `: Copy` struct declares a field of type `Heap<Int>`, `() -> Unit`, `compose () -> Unit` or `String?`
- **THEN** the type checker emits `E4016` naming that field

#### Scenario: Unknown type name is not Copy
- **WHEN** `struct Id : Copy { val v: UInt64 }` is declared and no struct or enum `UInt64` is in scope
- **THEN** the type checker emits `E4016` naming the field `v`

#### Scenario: Shared borrows are Copy
- **WHEN** a file declares `val LIMIT = 10`, `val REF: &Int = &LIMIT` and `fun f(): &Int = REF` at top level
- **THEN** the file typechecks successfully, because reading a `&Int` copies the borrow

#### Scenario: Copy-bounded type parameters are Copy
- **WHEN** `fun <T : Copy> get(x: &T): T = x` and `fun <T : Clone> take(x: &T): T = x` are checked
- **THEN** `get` typechecks successfully, copying `x` out of the borrow, and the type checker emits `E4017` for `take`

### Requirement: Built-In Clone Types
Every `Copy` type and `String` MUST be `Clone`, and `T?` and `Heap<T>` MUST be `Clone` exactly when `T` is. A type parameter MUST be `Clone` exactly when it is bounded by `Copy` or `Clone`. `&mut T`, function types, unbounded type parameters and type names with no declaration in scope MUST NOT be `Clone`.

#### Scenario: Clone built-ins as fields of a Clone struct
- **WHEN** `struct Doc : Clone { val title: String, val note: String?, val body: Heap<Int>, val size: Int }` is declared
- **THEN** the declaration typechecks successfully

#### Scenario: Non-Clone built-ins rejected in a Clone declaration
- **WHEN** a `: Clone` struct declares a field of type `() -> Unit` (`struct Task : Clone { val run: () -> Unit }`)
- **THEN** the type checker emits `E4016` naming that field

#### Scenario: Clone-bounded type parameters are Clone
- **WHEN** `fun <T : Clone> c(x: &Heap<T>): Heap<T> = x.clone()` is checked
- **THEN** the file typechecks successfully

### Requirement: Marker Declarations Are Checked Structurally
A `: Copy` declaration MUST be rejected with `InvalidMarkerDeclaration` (`E4016`) when any field or variant payload is not `Copy`, and a `: Clone` declaration when any is not `Clone`; its marker parameters, if generic, count as satisfying it. The diagnostic MUST name the marker, the field (or the variant and payload position) and its type, once per offending field. A local or imported struct or enum is `Copy` or `Clone` as its marker and, for a generic type, its type arguments say.

#### Scenario: Copy declaration with a non-Copy field rejected
- **WHEN** `struct User : Copy { val id: Int, val name: String, val tags: Heap<Int> }` is declared
- **THEN** the type checker emits `E4016` once for `name` and once for `tags`, and accepts `id`

#### Scenario: Copy enum with a non-Copy payload rejected
- **WHEN** `enum Event : Copy { Click(Int, Int), Text(String) }` is declared
- **THEN** the type checker emits `E4016` naming variant `Text`, its first payload and the type `String`

#### Scenario: Nested Copy types accepted
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `struct Rect : Copy { val origin: Point, val size: Point? }` are declared
- **THEN** both declarations typecheck successfully

#### Scenario: Field of a non-Copy struct rejected
- **WHEN** `struct Point { val x: Int }` (no marker) and `struct Rect : Copy { val origin: Point }` are declared
- **THEN** the type checker emits `E4016` naming `origin` and `Point`

#### Scenario: Clone declaration over Clone and Copy fields
- **WHEN** `struct Point : Copy { val x: Int }`, `struct User : Clone { val name: String }`, `struct Team : Clone { val lead: User, val at: Point }` and `struct Bad : Clone { val p: Raw }` with `struct Raw { val n: Int }` are declared
- **THEN** `Team` typechecks successfully and the type checker emits `E4016` for `Bad.p`

#### Scenario: Field of an imported marked type
- **WHEN** `geo/types` exports `public struct Point : Copy { val x: Int }`, `public struct Label : Clone { val text: String }` and `public struct Path { val name: String }`, and a file importing `/geo/types` declares `struct Seg : Copy { val a: Point }`, `struct Pin : Clone { val l: Label }` and `struct Bad : Copy { val l: Label }`
- **THEN** `Seg` and `Pin` typecheck successfully and the type checker emits `E4016` for `Bad.l`

#### Scenario: Generic fields of a marked declaration
- **WHEN** `struct Box<T> : Copy { val v: T }`, `struct Cell<T> : Clone { val v: T, val next: Heap<T>? }` and `struct Bad<T> : Copy { val b: Box<String> }` are declared
- **THEN** `Box` and `Cell` typecheck successfully, and the type checker emits `E4016` for `Bad.b`, because `Box<String>` is not `Copy`

### Requirement: Explicit Clone
Every `Clone` type MUST have a built-in `clone()` member taking no arguments, unless a field named `clone` takes precedence. It looks through `&` and `&mut` and returns an owned temporary of the receiver's type without those borrows (a receiver that is not a place is a temporary, see `references`). `x?.clone()` MUST work on nullables. On a type that is not `Clone`, `x.clone()` MUST call a visible extension named `clone`, or else report `NotCloneable` (`E4018`) naming the type.

#### Scenario: Cloning a field through a borrow
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: &User): String = u.name.clone()` is checked
- **THEN** the file typechecks successfully

#### Scenario: Cloning with safe navigation and elvis
- **WHEN** `struct User { val nick: String? }` is declared and `fun f(u: &User): String = u.nick?.clone() ?: "anonymous"` is checked
- **THEN** the file typechecks successfully

#### Scenario: Cloning through a safe navigation on a call result
- **WHEN** `struct User { val name: String }` and `fun u(): User? = User("a".clone())` are declared and `fun main() { println(u()?.name?.clone() ?: "none") }` runs with `--check-allocations`
- **THEN** it prints `a`, reports no live allocation and exits with status 0: the receiver of `clone()` is a `?.` chain of fields over the call's temporary, read in place as `u?.name?.clone()` is on a place

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

### Requirement: Consuming Positions
A value MUST be consumed when it initializes a local, a parameter default or a top-level `val`; is assigned, also through `*`; is passed to a by-value parameter, struct field or enum payload; is the receiver of a `consuming fun` call; or is returned (`return`, an expression body, a block or closure tail). Branches, block tails, parentheses and `?:` operands of a consuming position are consuming, as is a borrowed operand or receiver that is not a place (`references`). Other uses read in place.

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

#### Scenario: Branch and elvis operands are consumed
- **WHEN** `struct User { val name: String, val nick: String? }` is declared and a body of `fun f(c: Bool, u: User)` uses `(if (c) u.name else "x") == "a"` or `"a" + (u.nick ?: "x")` as an operand, `(u.nick ?: "x").length`, `when (u.nick ?: "x") { ... }` or `s += (if (c) u.name else "x")`
- **THEN** the type checker emits `E4017` for `u.name` and `u.nick`, because each operand is a temporary made by moving them, while the same expressions over owned locals run

#### Scenario: A field of a branch result is read from a temporary
- **WHEN** `struct User { val name: String }` is declared and `fun main() { val c = 1 > 0; val a = User("a".clone()); val b = User("b".clone()); println((if (c) a else b).name.clone()); val o: User? = null; println((o ?: User("x".clone())).name.length) }` runs with `--check-allocations`
- **THEN** it prints `a` and then `1`, reports no live allocation and exits with status 0: the branch and the `?:` are temporaries made by moving their operands, and the field is read from them

#### Scenario: Borrowing a safe navigation
- **WHEN** `struct User { val name: String, val id: Int }` is declared and a body of `fun f(u: User?)` runs `val r = &u?.name` and `val i = &u?.id`
- **THEN** the type checker emits `E4017` for `u?.name`, with help suggesting `u?.name?.clone()`, and accepts `&u?.id`, whose temporary is a copy

### Requirement: Moves Take Whole Values Only
A non-`Copy` value in a consuming position MUST be a whole owned local or by-value parameter, or a temporary (a call, constructor, literal or variant). Taking it from a field (also through a borrow, `Heap<T>` or a temporary), a safe navigation or a top-level `val` MUST be rejected with `CannotMoveOut` (`E4017`), naming the source. Its help MUST suggest a borrow, `.clone()` for a `Clone` type, and a destructuring declaration for a field of a whole owned local. The error type is never reported.

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

### Requirement: A Default Value Does Not Move Earlier Parameters
Inside a parameter's default value, an earlier parameter or `this` MUST NOT be moved, because the default only borrows them. Such a move MUST be rejected with `CannotMoveOut` (`E4017`) naming the parameter, with help suggesting `.clone()` for a `Clone` type.

#### Scenario: Moving an earlier parameter in a default value
- **WHEN** `struct User : Clone { val name: String }` is declared and `fun f(a: String, b: String = a): Int = 1`, `fun User.g(u: User = this): Int = 1`, `fun h(a: String, b: String = a.clone()): Int = 1` and `fun k(a: Int, b: Int = a): Int = b` are checked
- **THEN** the type checker emits `E4017` for `a` in `f`, naming the parameter `a` and suggesting `a.clone()`, and for `this` in `g`, and accepts `h` and `k`

### Requirement: A Closure Does Not Move Its Captures
Inside a closure, a non-`Copy` variable that the closure captures MUST NOT be moved, because the closure only borrows it (see `references`). Such a move MUST be rejected by the type checker with `CannotMoveOut` (`E4017`), naming the source "captured variable `s`, which the closure borrows", with help suggesting `.clone()` for a `Clone` type. A `Copy` capture MUST be copied, and a variable declared inside the closure MAY be moved.

#### Scenario: Moving a captured variable inside a closure
- **WHEN** `fun take(s: String) {}`, `fun run(f: () -> Int): Int = f()` and `struct Token { val id: Int }` are declared, and `fun f(s: String): Int = run { take(s); 1 }`, `fun g(t: Token): Int = run { val u = t; 1 }`, `fun h(s: String): Int = run { take(s.clone()); 1 }`, `fun k(n: Int): Int = run { val m = n; m }` and `fun l(): Int = run { val s = "a"; take(s); 1 }` are checked
- **THEN** the type checker emits `E4017` for `s` in `f`, naming the captured variable `s` and suggesting `s.clone()`, and for `t` in `g`, without a `.clone()` suggestion, and accepts `h`, `k` and `l`

#### Scenario: A closure in a default value moving an earlier parameter
- **WHEN** `fun take(s: String) {}` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(a: String, b: Int = run { take(a); 1 }): Int = b` is checked
- **THEN** the type checker emits `E4017` for `a`, naming the captured variable `a`

#### Scenario: Moving a captured variable after the closure ran
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(s: String) { run { greet(s) }; take(s) }` is checked
- **THEN** the file passes `prism check`, because the closure's borrow of `s` ends when `run` returns

### Requirement: No Mutable Top-Level State
Top-level declarations MUST be immutable: a top-level `var` MUST be rejected by the parser with `E2001`, whatever its visibility, with a message saying that mutable state belongs in locals and parameters (and, in UI code, in state handles). Top-level `val` declarations and local `var` declarations MUST be unaffected.

#### Scenario: Top-level var rejected
- **WHEN** a file declares `var counter = 0`, `public var counter: Int = 0` or `package var counter: Int = 0` at top level
- **THEN** the parser rejects the declaration with `E2001`, pointing at `var`

#### Scenario: Top-level val and local var accepted
- **WHEN** a file declares `val LIMIT = 10` at top level and `fun f(): Int { var x = LIMIT; x = x + 1; return x }`
- **THEN** the file typechecks successfully

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

### Requirement: Use After Move Is Rejected
A non-`Copy` value moved out of a local or parameter (by any consuming position) MUST NOT be used on a path on which it was moved, until it is assigned again. Reading, copying, moving, borrowing, calling or capturing it, or assigning one of its fields, MUST be rejected by the borrow checker with `UseAfterMove` (`E5001`). A value of a `Copy` type MUST be copied, never moved.

#### Scenario: Value move on assignment
- **WHEN** `fun take(s: String) {}` and `struct Tag { val s: String }` are declared, and a body of `fun f(a: String)` moves `a` by `val b = a`, by `take(a)` or by `val t = Tag(a)`, and then runs `take(a)`
- **THEN** the borrow checker emits `E5001` at `a` in the last `take(a)`

#### Scenario: Every use after a move is rejected
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared, and after `take(s)` a body of `fun f(s: String)` compares `s == "a"`, passes `greet(s)` (an implicit borrow), declares `val r = &s`, calls `s.clone()`, discards `s` as a statement, or runs `run { greet(s) }`, whose closure captures `s`
- **THEN** the borrow checker emits `E5001` at each of these uses of `s`

#### Scenario: Copy values are copied, not moved
- **WHEN** `struct Point : Copy { val x: Int }`, `fun g(p: Point) {}` and `fun h(n: Int) {}` are declared and `fun f(p: Point, n: Int) { g(p); g(p); val m = n; h(n); h(m) }` is checked
- **THEN** the file passes `prism check`

#### Scenario: Use after a consuming method call
- **WHEN** `struct User { val name: String }`, `struct Account { val owner: User }`, `consuming fun User.toAccount(): Account = Account(this)` and `fun greet(n: &String): Int = 1` are declared, and `fun f(u: User): Int { val a = u.toAccount(); return greet(u.name) }` or `fun g(u: User?) { val a = u?.toAccount(); val b = u?.toAccount() }` is checked
- **THEN** the borrow checker emits `E5001` at `u` in `u.name`, and at `u` in the second `u?.toAccount()`, which uses a possibly moved value

#### Scenario: A consuming receiver is moved before the arguments
- **WHEN** `struct Stack { var count: Int }`, `consuming fun Stack.into(n: Int): Int = n`, `fun bump(s: &mut Stack): Int = 1`, `mut fun Stack.push(n: Int) {}` and `fun Stack.size(): Int = count` are declared, and a body runs `var s = Stack(0); val k = s.into(bump(&mut s))`, or `var v = Stack(0); v.push(v.size())`
- **THEN** the borrow checker emits `E5001` at `&mut s`, because `s` moved into the receiver before `bump` was called, and accepts `v.push(v.size())`, whose receiver is borrowed after the arguments

#### Scenario: Conditional move
- **WHEN** `fun take(s: String) {}`, `fun check(s: String): Bool = true` and `fun g() {}` are declared, and a body of `fun f(c: Bool, n: Int, s: String)` moves `s` on one path only, by `if (c) take(s)`, by `when (n) { 1 -> take(s); else -> g() }` or by `val ok = c && check(s)`, and then runs `take(s)`
- **THEN** the borrow checker emits `E5001` at the last `take(s)`, saying that `s` is possibly moved

#### Scenario: Moves on separate paths are accepted
- **WHEN** `fun take(s: String) {}` is declared and a body of `fun f(c: Bool, s: String)` runs `if (c) take(s) else take(s)`, or `if (c) { take(s); return }` followed by `take(s)`
- **THEN** the file passes `prism check`, because no path moves `s` twice

#### Scenario: Loop move
- **WHEN** `fun take(s: String) {}` is declared and a body of `fun f(c: Bool, s: String)` runs `while (c) take(s)`, `for (i in 0..3) take(s)` or `do { take(s) } while (c)`
- **THEN** the borrow checker emits `E5001` at `s` in `take(s)`, whose related location is the same `take(s)` labelled as a move in a previous iteration of the loop

#### Scenario: A move followed by break is not repeated
- **WHEN** `fun take(s: String) {}` is declared and `fun f(c: Bool, s: String) { while (c) { take(s); break } }` is checked
- **THEN** the file passes `prism check`, because no iteration follows the move

#### Scenario: Plain enum is move-only without the marker
- **WHEN** `enum Color { Red, Green }` and `fun paint(c: Color) {}` are declared and `fun f(c: Color) { paint(c); paint(c) }` is checked, or the same with `enum Color : Copy { Red, Green }`
- **THEN** the borrow checker emits `E5001` at the second `c`, with help suggesting `: Copy` on the declaration of `Color` and no `.clone()`, and with the marker the file passes `prism check`

#### Scenario: Use of a String after it moved
- **WHEN** `fun take(s: String) {}` is declared and `fun f(a: String) { val b = a; take(a) }` and `fun g() { take("x"); take("x") }` are checked
- **THEN** the borrow checker emits `E5001` for `f`, with help suggesting `a.clone()` and no `: Copy`, and accepts `g`, because each string literal evaluates to a fresh owned `String`

### Requirement: Assignment Reinitializes a Moved Variable
Assigning a whole new value to a moved `var` MUST make it usable again from that assignment on, on the paths that pass through the assignment. Assigning one of its fields MUST NOT reinitialize it, and is a use after move.

#### Scenario: Reinitialization by assignment
- **WHEN** `fun take(s: String) {}` is declared and `fun f(a: String, b: String) { var s = a; take(s); s = b; take(s) }` is checked
- **THEN** the file passes `prism check`

#### Scenario: Reinitialization before the next iteration
- **WHEN** `fun take(s: String) {}` is declared and a body of `fun f(c: Bool, d: Bool)` runs `var s = "a"; while (c) { take(s); s = "b" }`, or `var t = "a"; while (c) { take(t); if (d) t = "b" }`
- **THEN** the borrow checker accepts the first loop and emits `E5001` at `t` in `take(t)`, which a previous iteration may have moved without reassigning it

#### Scenario: Assigning a field of a moved value
- **WHEN** `struct User { var name: String }` and `fun take(u: User) {}` are declared and a body runs `var u = User("a"); take(u); u.name = "b"`, or `var v = User("a"); take(v); v = User("b"); take(v)`
- **THEN** the borrow checker emits `E5001` at `u.name`, and accepts the assignment of the whole `v`

### Requirement: Use-After-Move Diagnostics Show the Moves
An `E5001` diagnostic MUST point at the use and name the value, and MUST list as related locations the moves that reach the use, each labelled "value moved here", with ", in previous iteration of loop" when only a loop's back edge leads from the move to the use. It MUST note the value's type, and help MUST suggest `.clone()` for a `Clone` type and `: Copy` for a struct or enum that could be `Copy`.

#### Scenario: Use after move points at the move
- **WHEN** `fun take(s: String) {}` is declared and `prism check` runs on `fun f(s: String) { take(s); take(s) }`
- **THEN** it reports `error[E5001]: use of moved value 's'` at the second `s`, labelled "value used here after move", with a related location at the first `s` labelled "value moved here", a note that `'s'` has type `'String'`, which is not `'Copy'`, and help suggesting `s.clone()`

#### Scenario: Possibly moved value
- **WHEN** `fun take(s: String) {}` is declared and `prism check` runs on `fun f(c: Bool, s: String) { if (c) take(s); take(s) }`
- **THEN** it reports `error[E5001]: use of possibly moved value 's'` at the second `s`, with a related location at the first `s`

#### Scenario: Labels point at the operands
- **WHEN** `struct Pair { val a: String, val b: String }`, `struct User { var name: String }` and `fun take(u: User) {}` are declared, and `fun f(s: String): Pair = Pair(s, s)`, `fun g(s: String): Bool { val t = s; return s == "a" }` or `fun h() { var u = User("a"); take(u); u.name = "b" }` is checked
- **THEN** the borrow checker emits `E5001` labelled at the operand that uses the value (the second `s`, the compared `s`, `u.name`), with its related location at the operand that moved it (the first `s`, the `s` of `val t = s`, the `u` passed to `take`), not at the whole construction, comparison or assignment

#### Scenario: Copy suggestion for a type whose members are all Copy
- **WHEN** `struct Point { val x: Int, val y: Int }`, `struct Name { val text: String }`, `fun g(p: Point) {}` and `fun h(n: Name) {}` are declared and `fun f(p: Point, n: Name) { g(p); g(p); h(n); h(n) }` is checked
- **THEN** the borrow checker emits `E5001` twice: for `p` with help suggesting `: Copy` on `Point`, and for `n` with neither a `: Copy` nor a `.clone()` suggestion

### Requirement: Reading an Uninitialized Value Is Rejected
A local MUST NOT be read on a path on which it has not been assigned. Such a read MUST be rejected by the borrow checker with `UseOfUninitialized` (`E5002`), with a related location at the jump that skips the local's declaration. Today the only such path is a `continue` in a `do`/`while` body whose condition reads a body local declared after the `continue`.

#### Scenario: Continue skips a declaration that the condition reads
- **WHEN** `fun next(): Int = 0` is declared and `fun f(c: Bool) { do { val s = "a"; if (c) continue; val x = next() } while (x > 0) }` is checked
- **THEN** the borrow checker emits `E5002` at `x` in the condition, saying that `x` is possibly uninitialized, with a related location at the `continue` labelled as skipping the declaration of `x`

#### Scenario: Condition reading only locals declared before the continue
- **WHEN** `fun next(): Int = 0` is declared and `fun f(c: Bool) { do { val x = next(); if (c) continue; val y = "b" } while (x > 0) }` is checked
- **THEN** the file passes `prism check`

### Requirement: Markers of Generic Types Are Conditional
A marker's parameters MUST be the type parameters that occur in a field or payload type of its declaration. An instantiation MUST be `Copy` when the marker is `Copy` and every marker parameter's argument is `Copy`, and `Clone` when the marker is `Copy` or `Clone` and every marker parameter's argument is `Clone`. The declaration check MUST treat each marker parameter as satisfying the marker.

#### Scenario: A Copy generic struct
- **WHEN** `struct Pair<A, B> : Copy { val a: A, val b: B }`, `struct User : Clone { val name: String }` and `struct Raw { val n: Int }` are declared, and `fun f(p: &Pair<Int, Int>): Pair<Int, Int> = p`, `fun g(p: &Pair<User, Int>): Pair<User, Int> = p.clone()`, `fun h(p: &Pair<String, Int>): Pair<String, Int> = p` and `fun k(p: &Pair<Raw, Int>): Pair<Raw, Int> = p.clone()` are checked
- **THEN** `f` and `g` typecheck successfully, the type checker emits `E4017` for `h`, because `Pair<String, Int>` is `Clone` but not `Copy`, and `E4018` for `k`

#### Scenario: A type parameter that no field uses does not condition the marker
- **WHEN** `struct Handle<T> : Copy { val id: Int }` is declared and `fun f(h: &Handle<String>): Handle<String> = h` is checked
- **THEN** the file typechecks successfully, because `Handle<String>` is `Copy`

#### Scenario: A marker parameter inside a non-Copy built-in
- **WHEN** `struct W<T> : Copy { val h: Heap<T> }` and `struct C<T> : Clone { val h: Heap<T>, val n: T? }` are declared
- **THEN** the type checker emits `E4016` for `W.h` and accepts `C`

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

#### Scenario: Taking apart through a safe navigation rejected
- **WHEN** `struct User { val name: String, val id: Int }` and `struct Box { val u: User? }` are declared and `fun f(b: Box?): Int = when (b?.u) { null -> 0; User(name = val s) -> 1 }` is checked
- **THEN** the type checker emits `E4017` for field `u` of `b`, with help suggesting a match on `&b` with a `null` branch, since `&b?.u` would move the field

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

## Design Rationale
Decisions from `value-semantics` (agreed with the user in the borrow-checker roadmap exploration and while proposing the change):

- **`Copy` and `Clone` are opt-in markers, and `Copy` implies `Clone`.** Each is a deliberate API promise, visible in the declaration and the ABI: adding a field never silently changes what a public type can do, and a unique token or a type that will own a resource is never duplicable by accident. Structural derivation (Swift's model) was rejected because it would need opt-out markers later. `Copy` means an implicit bitwise copy that never allocates; `Clone` means an explicit `.clone()` that may allocate, which is why `String` is `Clone` but not `Copy`: an implicit deep copy would be a hidden allocation, and a hidden reference count is forbidden by the zero-cost memory-safety invariant. Rust's requirement to write both was rejected as buying nothing without custom clone bodies.
- **The predicates are shallow.** A struct or enum is `Copy` or `Clone` exactly as its marker says, whatever its fields, so neither the predicates nor the structural check recurse through type definitions or need a cycle guard. The structural check runs only in the defining file; importers trust the marker recorded in the ABI.
- **The built-in tables.** `&mut T` and function types are neither: a closure holds inferred borrows of its captures, which a copy or clone would duplicate. Type parameters are neither until bounds exist. Unknown (opaque) type names are neither, because nothing has promised that they can be duplicated; this row disappears once unknown names are rejected. The error type counts as both, so one mistake is not reported twice.
- **One marker map in the checker.** The marker of every nominal type comes from the file's own declarations, its imports' exported types, and every nominal reference in an ingested ABI, which covers types whose module the file does not import. Putting the marker on `Type` was rejected: two values of one type could disagree.
- **`clone()`, not `copy()`.** Kotlin's data-class `copy()` is a shallow copy with named overrides, so a deep `copy()` would mislead Kotlin users and would take the name from a possible struct-update feature. `clone` is a built-in member rather than a library function, because methods do not exist yet and a user-written `clone` could not be called generically without traits. Its receiver is read in place through `&` and `&mut`, and its result is a temporary. A user type's clone clones each field; custom clone bodies wait for traits. Deferring cloning until traits was rejected because whole-value moves would leave no way to get an owned copy of a field.
- **A field named `clone` wins over the built-in** (user). This matches Kotlin, where members, including a property of function type, take precedence over extensions. Making the built-in always win was rejected: the parser drops parentheses, so `(x.clone)()` cannot be told apart from `x.clone()`, and preserving them would add an expression node for an edge case. `extension-methods` can apply the same rule to every field-versus-extension clash, and must reject a user extension named `clone` on a `Clone` type.
- **Moves take whole values only.** A value is either whole or gone, which is simpler to explain and to drop, and keeps the future move checker to whole locals. Fields of temporaries are rejected too (`make().name`), so a temporary never needs a partial drop, which matters once destructors exist (Rust forbids it for `Drop` types). `?:` and safe navigation consume, since the result is the field's value. Partial moves and moves out of temporaries remain possible relaxations that break no program. Every case is decided by the expression's shape and type, so it is a type-checker rule.
- **A consuming flag, not a separate pass.** A flag set at consuming sites (initializers, assignments, arguments, returns) and handed through branches, block tails and `?:` operands is taken by the next expression checked, so the leaf check runs where the leaf's type and scope are known. A post-pass would re-infer expressions or resolve names after their scope has closed.
- **One move-out code for every source.** `E4017` names the source ("field 'name' of 'u'", "top-level value 'GREETING'"), and `reference-access` adds "borrow" to it rather than taking a new code, as Rust uses one E0507 for every place behind a reference. Its help suggests a borrow and, for a `Clone` type, `.clone()`.
- **Top-level `var` is removed in the parser.** A borrow of a mutable global created in one function can be invalidated by an assignment in a callee, which no single function's analysis sees; Rust makes `static mut` unsafe for this reason. Rejecting it in the parser leaves no `var` flag in the AST or ABI, following the precedent of borrowed struct fields; a type-checker code would have kept a flag that no valid program sets.

Decisions from `reference-access`:

- **Moving out of a borrow reuses `E4017`** (user). The source is "borrow `u`", as Rust uses one E0507 for every place behind a reference, and the fixes are the same as for a field. The help suggests `u.clone()` for a `Clone` type, and otherwise taking a `&T` where the value is used, never `&u`, which is already a borrow.
- **A borrow of a non-place consumes its operand** (user). `&(if (c) u.name else v.name)`, `&(u.nick ?: "x")` and `&u?.name` borrow a temporary made by moving the field, so they report `E4017`. A safe navigation is not a place, so `E4017`'s help for one does not suggest borrowing it.

Decisions from `extension-methods`:

- **A `consuming fun` receiver is one more consuming position** (user, from `value-semantics`). It is checked exactly like a by-value argument, so `E4017`'s help stays the same for every consuming site: borrow, clone, or take the whole value. Through a reference, a `Copy` receiver is copied out and any other is a move out of the borrow; through boxes alone it is unboxed (`heap-boxes`, below). Because moves take whole values, a `consuming fun` passes `this` on or takes it apart with `val (...) = this` (`payload-patterns`).
- **An extension named `clone` is called only on a type that is not `Clone`.** On a `Clone` type the built-in always wins, so such an extension is rejected (see `extension-functions`).

Decisions from `borrowck-moves`:

- **Flow-sensitive rules are the borrow checker's, and live here** (user for the split). Whether a value still holds a value depends on the path taken through branches and loops, so use after move is a borrow-checker error (`E5xxx`) on MIR, while a rule decided by an expression's shape and type stays a type error (`E4xxx`). The rules join this spec rather than a `borrow-checking` capability, so each concept stays in one spec whatever stage enforces it.
- **Move paths are a local plus fields; a payload counts as its nullable.** Moving `(u as payload)`, as `u?.toAccount()` and `?:` do, moves `u`: a `T?` owns nothing but its payload, so no partial-move state is needed and drops stay exact. So `u == null` after a conditional payload move is a use of a possibly moved value, as in Rust, until smart casts exist. A dereference or an index ends a path; a `Copy` value never moves. Field child paths are kept for later partial moves and disjoint borrows.
- **Two forward analyses over one dataflow framework.** `MaybeInitialized` (paths that may hold a value) and `ReachingUninitializations` (the moves and storage starts that reach a point unreinitialized) together tell definitely, possibly and never initialized apart, and the reaching events are what the diagnostic labels. The framework is generic, forward and backward, so `borrowck-exclusivity` adds liveness and borrows on it.
- **What a use is.** Reads, copies, moves, borrows, discriminant reads, calls of a function value, closure captures and field writes use their place; a whole-local assignment reinitializes it. Drops are not uses: whether a drop of a moved value runs is exactly what elaboration decides.
- **Two codes, because the fixes differ.** `E5001` (clone, copy or reassign the value) and `E5002` (move the declaration before the jump that skips it), as Rust's E0382 and E0381. A use reached by the same moves as an already reported use of the same value is not reported again. Today only a `continue` in a `do`/`while` can reach an uninitialized read; deferred `val` initialization will reuse `E5002`.
- **"In previous iteration of loop" comes from back edges.** A move is labelled so when it reaches the use only through a back edge (an edge to a dominator of its source), computed without back edges in a second solution.
- **Labels point at operands.** Each MIR operand records the span of the source that reads it, so both moves of `Pair(s, s)` and the compared `s` of `s == "a"` are labelled where they are written, not at their statement.
- **Help suggests `: Copy` only when it would compile.** A struct or enum without the marker whose every field or payload is `Copy` gets the `: Copy` suggestion (naming its module when imported), a `Clone` type gets `.clone()`, and a `var` moved in a loop the reassignment. Rust's "consider borrowing here" was rejected: a parameter is often by-value on purpose, and the callee may be in another file.
- **A default value may not move an earlier parameter.** Its generator only borrows the earlier arguments and `this`, so moving one would move out of a borrow; this depends only on the expression's shape, so it is `E4017` in the type checker, not a borrow-checker code.
- **Closures.** Building a closure uses its captures; a closure's body is checked on its own, its captures initialized on entry. A closure never moves a capture, and moving a variable while a closure capturing it is live is a move while borrowed (`borrowck-exclusivity`, below and in `references`).

Decisions from `borrowck-exclusivity`:

- **A closure does not move its captures.** A closure borrows what it captures (`references`), so moving a capture inside it is a move out of a borrow. Whether an expression moves a captured binding depends only on its shape and on the closure nesting, so it is `E4017` in the type checker, like a default value moving an earlier parameter, with the source "captured variable 's', which the closure borrows": this settles `run { take(s) }; take(s)`, which passed and dropped `s` twice. Rejected: a borrow-checker code, and accepting the move as a move capture, which needs the escaping-closure design of `closures` (a closure owning `s` could run at most once).

Decisions from `generics` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Markers of generic types are conditional on the type parameters their fields use.** For `struct Pair<A, B> : Copy`, the marker parameters are the type parameters that occur syntactically in a field or payload type, read from the AST alone so Phase 1 can record them. `Pair<X, Y>` is `Copy` when every marker parameter's argument is, and the declaration check treats each marker parameter as satisfying the marker, so `struct Box<T> : Copy { val v: T }` is accepted and a phantom `struct Handle<T> : Copy { val id: Int }` is `Copy` for every `T`, which `State<T>` needs. Rejected: unconditional markers requiring bounds (no `Box<String>` could exist), Rust's derive rule conditioning on every parameter (a phantom handle would not be `Copy`), and a semantic condition from the field types' own markers, which needs imported marker parameters in Phase 1a; the syntactic rule only ever errs towards "not `Copy`".

Decisions from `payload-patterns` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Taking a value apart moves it whole** (D6). A branch or destructuring declaration that binds a non-`Copy` part by value moves its whole subject on that path, so the borrow checker sees one whole-value move plus one initialization per part, and nothing is ever partly moved or partly dropped. The source must be one a move already accepts (a whole owned local, a by-value parameter, a consuming `this`, a temporary); a field, a borrow's referent, a top-level `val`, a capture or a branch, block or `?:` expression is `E4017`, with help suggesting the borrow to match on (for a safe navigation, the borrow of what `?.` reads). A branch expression is rejected rather than consumed because the subject is checked before its patterns decide whether it moves.
- **Partial moves stay out of the language** (D7), closing the "possible relaxation" left by `value-semantics`. Every use it had is covered by taking the value apart: `val (name, email) = user` instead of moving `user.name` then reading `user.email`, `val (name) = this` in a `consuming fun`, a `when` that moves a payload. Partial moves would make drop elaboration track fields (field-level drop flags, partial drops of the remainder) and make every later use ask which fields are left. `E4017`'s help for a field of a whole owned local suggests the destructuring. Revisit only if real programs show patterns destructuring cannot express.
- **A use after taking apart is labelled at the binding** (D16, D24). `E5001` points at the binding that moved a part, and when the moved value was destructured its help suggests matching on its borrow (`when (&m)` or `val (...) = &m`).

Decisions from `heap-boxes` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **Moving out is explicit: the consuming built-in `h.unbox()`** (D6). It moves the contents out and frees the box without dropping them, following the move rules of every consuming receiver (a field or a top-level `val` is `E4017`, a box behind a reference is a move out of the borrow, with help suggesting `.clone().unbox()`), works through `?.` (`n?.unbox()` is a `T?`) and peels one box. A non-`Copy` content where a `T` is expected (`val u: User = h`) is `E4017`, "the contents of `Heap` `h`", with help suggesting `h.unbox()`, mirroring a move out of a borrow; a `Copy` content is copied out. Rejected: Rust's move out of `*b` (no read `*`); implicit unboxing at by-value positions (a type mismatch would silently free a box, and `val x: T = h` would read unlike `val x = h`); a free function; the names `take()`, `get()` and `into()`.
- **A `consuming fun` receiver reached through boxes is unboxed** (D7), once per box layer, as Rust's method calls move out of a `Box`: receivers look through boxes in every other mode with nothing written, and the source is consumed either way. The receiver must be movable as a whole; any reference layer keeps it a move out of a borrow (`E4017`). `h.unbox().f()` stays possible. Rejected: keeping `E4017` (it made `consuming` the one mode that does not look through boxes); unboxing through references.
- **Branch and `?:` operands are consumed** (from the review). An `if`, `when`, block or `?:` used as an operand, a `when` subject or a member access target is a temporary, so a place in its branches is moved, as the rule above already said; the checker had read them in place, which MIR could not lower (a crash on `"a" + (k.t ?: "-")`).
