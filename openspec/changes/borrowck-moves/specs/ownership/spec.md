# Ownership Specification Delta

## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: Moves Take Whole Values Only
A non-`Copy` value in a consuming position MUST be a whole owned local or by-value parameter, or a temporary (a call, constructor, literal or variant). Taking it from a field (also through a borrow, `Heap<T>` or a temporary), a safe navigation or a top-level `val` MUST be rejected with `CannotMoveOut` (`E4017`), naming the source. Inside a parameter's default value, an earlier parameter or `this` MUST NOT be moved either, because the default only borrows them; this is rejected with `E4017` naming the parameter. Its help MUST suggest a borrow and, for a `Clone` type, `.clone()`. The error type is never reported.

#### Scenario: Moving a non-Copy field rejected
- **WHEN** `struct User { val id: Int, val name: String }` is declared and a body of `fun f(u: User)` declares `val n = u.name`
- **THEN** the type checker emits `E4017` stating that `name` cannot be moved out of `u`, with help suggesting `&u.name` or `u.name.clone()`, while `val i = u.id` in the same body is accepted

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

#### Scenario: Moving an earlier parameter in a default value
- **WHEN** `struct User : Clone { val name: String }` is declared and `fun f(a: String, b: String = a): Int = 1`, `fun User.g(u: User = this): Int = 1`, `fun h(a: String, b: String = a.clone()): Int = 1` and `fun k(a: Int, b: Int = a): Int = b` are checked
- **THEN** the type checker emits `E4017` for `a` in `f`, naming the parameter `a` and suggesting `a.clone()`, and for `this` in `g`, and accepts `h` and `k`
