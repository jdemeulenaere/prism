# References Specification

## Purpose
The references specification defines how borrowed references are used: as the type checker enforces it, where borrows are created implicitly, how values are read and written through references, how reference bindings are re-pointed and reborrowed, how borrowed nullables are viewed, and how reference types relate; and as the borrow checker enforces it, how long each borrow lasts, which accesses conflict with it, what a call's result keeps borrowed, how closures borrow what they capture, and why a capturing closure cannot escape. It also defines the four lifetime elision rules, which tie a returned borrow to inputs from the signature alone, for callers (what a call's result keeps borrowed) and callees (what a body may return or store through its inputs).

## Requirements

### Requirement: Implicit Shared Borrow at Argument Sites
Where a call's parameter has type `&T`, an argument of type `T` MUST be borrowed implicitly. A place argument (a variable or a `.` field path) MUST be borrowed where it lives, without being consumed. A string's character `s[i]` is a value, not a place. Any other argument MUST be evaluated, in a consuming position, into a temporary of the referent type that lives until the end of the statement. Default values are arguments. Elsewhere a borrow MUST be written with `&`.

#### Scenario: Implicit shared borrow at argument sites
- **WHEN** `fun greet(name: &String)` and `struct User { val name: String }` are declared and a body of `fun f(u: &User, name: String)` runs `greet(name)`, `greet(u.name)` and `greet("Ada")`
- **THEN** the body typechecks successfully, and neither `name` nor `u.name` is reported as moved

#### Scenario: Borrowing the result of a call
- **WHEN** `fun make(): String` and `fun greet(name: &String)` are declared and `fun f() { greet(make()) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: A branch result is a temporary
- **WHEN** `fun greet(name: &String)` and `struct User { val name: String }` are declared, and `fun f(c: Bool, u: User, v: User) { greet(if (c) u.name else v.name) }` and `fun g(c: Bool) { greet(if (c) "a" else "b") }` are checked
- **THEN** the type checker emits `E4017` for `u.name` and for `v.name`, and accepts `g`

#### Scenario: A temporary is converted before it is borrowed
- **WHEN** `fun show(n: &Int?): Int { val m: Int? = n; return m ?: 0 }` is declared and `fun main() { println(show(5)); println(show(null)) }` runs
- **THEN** it prints `5` and then `0`: the argument `5` is wrapped as an `Int?` and the borrow points at the wrapped value

#### Scenario: A default value is borrowed like an argument
- **WHEN** `fun greet(name: &String = "guest")` is declared and `fun f() { greet() }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Calling a function-typed value
- **WHEN** `fun f(g: (&String) -> Unit, s: String) { g(s) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Borrows outside arguments are written
- **WHEN** `struct User { val name: String }` is declared, and `fun f(name: String) { val r: &String = name }` and `fun g(u: &User): &String = u.name` are checked
- **THEN** the type checker emits `E4002` for `f`, with help suggesting `&name`, and `E4005` for `g`, with help suggesting `&u.name`

#### Scenario: A string character is borrowed as a temporary
- **WHEN** `fun show(c: &Char): Int = 1` is declared and `fun f() { var s = "ab"; val r = &s[0]; s = "cd"; println(show(s[1])); println(r) }` is checked
- **THEN** the file typechecks successfully: `r` and the argument of `show` borrow temporaries holding the characters, not `s`

### Requirement: Explicit Mutable Borrows at Call Sites
A parameter of type `&mut T` MUST NOT borrow an owned argument implicitly. The caller MUST write `&mut`, and an owned place passed without it MUST be rejected with `E4004`, with help suggesting `&mut` before the argument. The receiver of a `mut fun` call is the one exception: it is borrowed mutably without `&mut` (see "Mutable Receivers Must Be Mutable Places").

#### Scenario: Explicit mutation at call sites
- **WHEN** `fun bump(r: &mut Int)` is declared, and a body declares `var x = 1` and calls `bump(x)` and `bump(&mut x)`
- **THEN** the type checker emits `E4004` for `bump(x)`, with help suggesting `&mut x`, and accepts `bump(&mut x)`

### Requirement: Passing a Mutable Borrow Reborrows It
A place of type `&mut T` passed as an argument where `&mut T` or `&T` is expected, or a place of type `(&mut T)?` passed where `(&mut T)?` or `(&T)?` is expected, MUST be reborrowed for the call rather than moved, so that it stays usable afterwards. `&mut` applied to a `val` binding MUST be rejected with `E4003`, whatever the binding's type.

#### Scenario: Passing a mutable borrow twice
- **WHEN** `fun bump(r: &mut Int)` and `fun read(r: &Int): Int` are declared and `fun f(r: &mut Int): Int { bump(r); bump(r); return read(r) }` is checked
- **THEN** the file typechecks successfully, and each call reborrows `r` without `&mut`

#### Scenario: Passing an optional mutable borrow twice
- **WHEN** `struct User { var name: String }`, `fun edit(m: (&mut User)?) {}` and `fun both(a: (&mut User)?, b: (&mut User)?) {}` are declared, and `fun f(m: (&mut User)?) { edit(m); edit(m) }`, `fun g(u: User) { var p = u; val m: (&mut User)? = &mut p; edit(m); edit(m) }` and `fun h(m: (&mut User)?) { both(m, m) }` are checked
- **THEN** the borrow checker accepts `f` and `g`, where each call reborrows `m` and `m` stays usable, and emits `E5003` at the second `m` in `both(m, m)`, two mutable reborrows passed to one call

#### Scenario: Mutable borrow of a val reference binding rejected
- **WHEN** `fun f(r: &mut Int) { val m = &mut r }` is checked
- **THEN** the type checker emits `E4003`, because `&mut r` borrows the `val` binding `r` itself

### Requirement: Copy-Out From References
Where a value of type `T` is expected and the expression has a reference or box type (`&T`, `&mut T`, `Heap<T>` or nested borrows and boxes of `T`), a `Copy` value MUST be copied out implicitly, at every value position: initializers with a type, assigned values, returned values, by-value arguments, constructor fields and payloads, and default values. `(&T)?` MUST be accepted where `T?` is expected when `T` is `Copy`. An unannotated initializer MUST keep the reference or box type.

#### Scenario: Copy-out of a borrowed primitive
- **WHEN** `fun g(x: Int): Int` is declared and `fun f(a: &Int): Int = g(a)` is checked
- **THEN** the call typechecks, copying the `Int` out of the borrow

#### Scenario: Copy-out at every value position
- **WHEN** `struct Point : Copy { val x: Int }` and `struct Line { val start: Point }` are declared and a body of `fun f(a: &Int, p: &mut Point): Point` runs `val x: Int = a`, `var y = 0`, `y = a`, `val l = Line(p)` and `return p`
- **THEN** the body typechecks successfully

#### Scenario: Copy-out through nested and nullable borrows
- **WHEN** `fun f(a: &&Int): Int = a` and `fun g(a: (&Int)?): Int? = a` are checked
- **THEN** both typecheck successfully

#### Scenario: An unannotated initializer copies the borrow
- **WHEN** `fun f(a: &Int): &Int { val b = a; return b }` is checked
- **THEN** the file typechecks successfully, and `b` has type `&Int`

#### Scenario: Copy-out of a box's contents
- **WHEN** `struct Point : Copy { val x: Int }` is declared and `fun f(h: Heap<Int>, r: &Heap<Point>): Int { val n: Int = h; val p: Point = r; return h }` and `fun g(h: Heap<Int>): Heap<Int> { val k = h; return k }` are checked
- **THEN** both typecheck successfully, `h` stays usable in `f` after each copy, and `k` has type `Heap<Int>`, a move of the box

### Requirement: Writing Through a Mutable Reference
`*r = v` MUST write `v` into the place `r` refers to, and `*h = v` into the contents of the box `h`, each `*` peeling one reference or box layer. Every reference layer written through MUST be `&mut` and every box layer a mutable place, or it is `E4003`. A `*` of a value that is neither a reference nor a box MUST be `NotAReference` (`E4019`). The value MUST be checked against the referent type in a consuming position; the binding holding a reference MAY be a `val`.

#### Scenario: Writing through a mutable borrow
- **WHEN** `fun bump(r: &mut Int) { *r = r + 1 }` is checked
- **THEN** it typechecks and writes through `r`; `r = 5` in the same body is rejected with `E4003`, because `r` is a `val` binding of reference type, with help suggesting `*r = 5`

#### Scenario: Writing through nested borrows
- **WHEN** `fun f(rr: &mut &mut Int) { **rr = 0 }` and `fun g(rr: &(&mut Int)) { **rr = 0 }` are checked
- **THEN** `f` typechecks successfully and the type checker emits `E4003` for `g`, because the outer borrow is shared

#### Scenario: Writing through a shared borrow rejected
- **WHEN** `fun f(r: &Int) { *r = 5 }` is checked
- **THEN** the type checker emits `E4003`, stating that a shared borrow cannot be written through

#### Scenario: Writing through a value that is not a reference
- **WHEN** `fun f() { var x = 1; *x = 5 }` is checked
- **THEN** the type checker emits `E4019` naming the type `Int`, saying that `*` needs a reference or a `Heap`

#### Scenario: The written value is checked and consumed
- **WHEN** `struct User { val name: String }` is declared and a body of `fun f(r: &mut String, a: &Int, n: &mut Int, s: String, u: User)` runs `*n = a`, `*r = s`, `*r = u.name` and `*n = "x"`
- **THEN** `*n = a` (a copy-out) and `*r = s` (a move of a whole parameter) are accepted, the type checker emits `E4017` for `u.name`, and it emits `E4002` for `"x"`

#### Scenario: Writing through the result of a call
- **WHEN** `fun pick(a: &mut Int, b: &mut Int): &mut Int` is declared and a body declares `var a = 1` and `var b = 2` and runs `*pick(&mut a, &mut b) = 0`
- **THEN** the body typechecks successfully

#### Scenario: Writing a box's contents
- **WHEN** `fun f(h: Heap<Int>, r: &mut Heap<Int>): Int { var m = h; *m = 2; *m += 1; **r = 3; *r = Heap(4); return m }` is checked
- **THEN** the body typechecks successfully: `*m = 2` and `*m += 1` write the contents of `m`, `**r = 3` the contents of the box `r` refers to, and `*r = Heap(4)` replaces that box

#### Scenario: Writing through a box that is not mutable rejected
- **WHEN** `struct Holder { val h: Heap<Int> }` is declared and `fun f(h: Heap<Int>) { *h = 1 }`, `fun g(r: &Heap<Int>) { **r = 1 }` and `fun k(o: Holder) { var p = o; *p.h = 1 }` are checked
- **THEN** the type checker emits `E4003` for each, with help suggesting a `var` binding for `f`, a `&mut` borrow for `g`, and a `var` field for `k`

#### Scenario: Assigning the contents' type to a box rejected
- **WHEN** `fun f(h: Heap<Int>) { var m = h; m = 5 }` is checked
- **THEN** the type checker emits `E4002`, with help suggesting `*m = 5`

### Requirement: Re-Pointing Reference Variables
Assigning to a `var` binding of reference type MUST re-point it to the assigned reference without writing to any referent. Assigning a value of the referent's type to such a binding MUST be rejected (`E4002` for a `var`, `E4003` alone for a `val`), with help suggesting `*r = v` when the reference is `&mut`.

#### Scenario: Re-pointing a reference variable
- **WHEN** a body declares `var r: &Int = &a` and later assigns `r = &b`
- **THEN** `r` refers to `b`, and neither `a` nor `b` is modified

#### Scenario: Assigning a value to a mutable reference variable
- **WHEN** a body declares `var x = 1`, `var y = 2` and `var r: &mut Int = &mut x`, then assigns `r = 5` and `r = &mut y`
- **THEN** the type checker emits `E4002` for `r = 5`, with help suggesting `*r = 5`, and accepts `r = &mut y`

### Requirement: Borrowed Nullables Expose Their Contents
`&(T?)` MUST convert implicitly to `(&T)?`, and `&mut (T?)` to `(&mut T)?` or `(&T)?`, wherever such a type is expected. Safe navigation and `?:` MUST read through references to a nullable as if they were `(&T)?`. A place of type `T?` passed to a `(&T)?` parameter MUST be borrowed where it lives. `(&T)?` MUST NOT convert to `&(T?)`.

#### Scenario: Borrowing a nullable field without cloning
- **WHEN** `struct User { val nick: String? }`, `fun greet(n: &String)` and `fun show(n: (&String)?)` are declared and a body of `fun f(u: &User)` runs `show(u.nick)`, `greet(&u.nick ?: &"anon")` and `val a: (&String)? = &u.nick`
- **THEN** the body typechecks successfully, and `u.nick` is not reported as moved

#### Scenario: Elvis over a borrowed nullable
- **WHEN** `fun greet(n: &String)` is declared, and `fun f(n: &String?) { greet(n ?: &"anon") }` and `fun g(n: &String?) { greet(n ?: "anon") }` are checked
- **THEN** `f` typechecks successfully, and the type checker emits `E4002` for `g`, because `"anon"` is a `String` and the left side is a `&String`

#### Scenario: Safe navigation through a borrowed nullable
- **WHEN** `struct User { val id: Int }` is declared and `fun f(n: &User?): Int? = n?.id` and `fun g(n: &User?): Bool = n == null` are checked
- **THEN** both typecheck successfully

#### Scenario: Mutable view of a borrowed nullable
- **WHEN** `fun set(r: (&mut Int)?)` and `fun read(r: (&Int)?)` are declared and `fun f(m: &mut Int?) { set(m); read(m) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Optional borrow does not become a borrowed nullable
- **WHEN** `fun g(r: &String?)` is declared and `fun f(r: (&String)?) { g(r) }` is checked
- **THEN** the type checker emits `E4004`

### Requirement: Reference Types Are Invariant
`&U` MUST be assignable to `&T` only when `U` and `T` are the same type, or differ only in that a reference inside `U` is `&mut` where `T` has `&`. `&mut U` MUST be assignable to `&mut T` only when `U` and `T` are the same type. `&mut U` MUST be assignable to `&T` whenever `&U` is.

#### Scenario: A borrowed value is not a borrowed nullable
- **WHEN** `fun g(r: &Int?)` is declared and `fun f(a: &Int) { g(a) }` is checked
- **THEN** the type checker emits `E4004`, because `&Int` and `&(Int?)` refer to values of different layouts

#### Scenario: Mutable borrows weaken to shared ones
- **WHEN** `fun read(r: &Int): Int` and `fun peek(rr: &&Int): Int` are declared and `fun f(r: &mut Int, rr: &(&mut Int)): Int { return read(r) + peek(rr) }` is checked
- **THEN** the file typechecks successfully

### Requirement: Method Receivers Borrow Implicitly
The receiver of an extension call MUST be passed by the extension's receiver mode with nothing written at the call site: borrowed shared for `fun`, borrowed mutably for `mut fun`, and by value for `consuming fun` (see `ownership`). Any number of `&`, `&mut` and `Heap<T>` layers around the receiver MUST be looked through. A place MUST be borrowed where it lives, and any other receiver MUST be a temporary, in a consuming position, that lives until the end of the statement.

#### Scenario: Shared receivers of places, references and temporaries
- **WHEN** `struct User { val name: String }`, `fun User.greet(): String`, `fun String.shout(): String` and `fun make(): User` are declared, and a body of `fun f(u: User, r: &User, rr: &&User, m: &mut User, h: Heap<User>)` runs `u.greet()`, `u.name.shout()`, `r.greet()`, `rr.greet()`, `m.greet()`, `h.greet()`, `make().greet()` and `"a".shout()`
- **THEN** the body typechecks successfully, and neither `u` nor `u.name` is reported as moved

#### Scenario: A branch receiver is a temporary
- **WHEN** `fun String.shout(): String` and `struct User { val name: String }` are declared and `fun f(c: Bool, u: User, v: User): String = (if (c) u.name else v.name).shout()` is checked
- **THEN** the type checker emits `E4017` for `u.name` and for `v.name`

### Requirement: Method-Call Receivers Are Classified Before Resolution
The receiver of every call `value.name(...)` MUST be classified before the member it calls is resolved, since that depends on the receiver's type: a place, or a `?.` chain of fields over one, is read in place, and anything else is a temporary in a consuming position. This applies to a function-typed field call and to the built-in `clone()` as to an extension call.

#### Scenario: A branch receiver of clone is a temporary
- **WHEN** `struct User { val name: String }` is declared and `fun g(c: Bool, u: User, v: User): String = (if (c) u.name else v.name).clone()` is checked
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

### Requirement: Shared Borrows Coexist
Any number of shared borrows of a value MUST be allowed to be live at the same time, together with reads of the value through its owner.

#### Scenario: Concurrent shared borrows
- **WHEN** `fun both(a: &String, b: &String): Int = 1` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String): Int { val r = &s; val t = &s; greet(s); return both(r, t) + both(s, s) }` is checked
- **THEN** the file passes `prism check`

### Requirement: A Mutable Borrow Is Exclusive
While a mutable borrow of a place is live, the borrow checker MUST reject any other borrow of an overlapping place and any read of it through its owner with `BorrowConflict` (`E5003`). While a shared borrow is live, it MUST reject a mutable borrow of an overlapping place with `E5003`. A closure capture and the implicit borrow of an argument or a receiver are borrows.

#### Scenario: Exclusive mutable borrow prevents concurrent access
- **WHEN** `fun print(n: &Int) {}` and `fun bump(r: &mut Int) {}` are declared and a body declares `var x = 1` and then runs `val m = &mut x; val k = &mut x; print(m)`, or `val m = &mut x; print(x); print(m)`, or `val r = &x; bump(&mut x); print(r)`
- **THEN** the borrow checker emits `E5003` at the second `&mut x`, at `x` in `print(x)`, and at `&mut x` in `bump(&mut x)`

#### Scenario: A mutable receiver conflicts with a live shared borrow
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}` and `fun print(n: &Int) {}` are declared and a body runs `var v = Stack(0); val c = &v.count; v.push(1); print(c)`
- **THEN** the borrow checker emits `E5003` at the receiver `v` of `push`

#### Scenario: Two mutable borrows passed to one call
- **WHEN** `struct User { var name: String }` and `fun touch(a: &mut User, b: &mut String) {}` are declared and a body runs `var u = User("a"); touch(&mut u, &mut u.name)`
- **THEN** the borrow checker emits `E5003` at `&mut u.name`

### Requirement: Borrowed Values Cannot Be Assigned or Moved
While a borrow of a place is live, the borrow checker MUST reject an assignment to an overlapping place with `AssignToBorrowed` (`E5004`), and a move of an overlapping place with `MoveOutWhileBorrowed` (`E5005`). Re-pointing a reference variable MUST NOT count as an assignment to what it refers to.

#### Scenario: Assigning a borrowed value
- **WHEN** `struct User { var name: String }`, `fun print(n: &Int) {}` and `fun greet(s: &String): Int = 1` are declared, and a body runs `var x = 1; val r = &x; x = 2; print(r)`, or `var u = User("a"); val n = &u.name; u.name = "b"; greet(n)`, or `var w = User("a"); val v = &w; w.name = "b"; greet(v.name)`
- **THEN** the borrow checker emits `E5004` at `x` in `x = 2`, at `u.name` in the assignment, and at `w.name` in the assignment

#### Scenario: Moving a borrowed value
- **WHEN** `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared and `fun f(s: String) { val r = &s; take(s); greet(r) }` is checked
- **THEN** the borrow checker emits `E5005` at `s` in `take(s)`, with help suggesting `s.clone()`

### Requirement: Borrows Last Until Their Last Use
A borrow MUST be live from where it is created until the last use of every reference that holds it, on each path, and not until the end of its block. A use in a later iteration of a loop MUST keep the borrow live around the loop.

#### Scenario: Borrow ends at its last use
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { var x = 1; val r = &x; print(r); x = 2 }` is checked
- **THEN** the file passes `prism check`, because `r` is not used after `print(r)`

#### Scenario: A borrow used on one path only
- **WHEN** `fun print(n: &Int) {}` is declared, and `fun f(c: Bool) { var x = 1; val r = &x; if (c) print(r); x = 2 }` and `fun g(c: Bool) { var x = 1; val r = &x; if (c) x = 2; print(r) }` are checked
- **THEN** the borrow checker accepts `f` and emits `E5004` at `x` in `x = 2` of `g`

#### Scenario: A borrow used in a later iteration
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f(c: Bool) { var x = 1; val r = &x; while (c) { print(r); x = 2 } }` is checked
- **THEN** the borrow checker emits `E5004` at `x` in `x = 2`, whose borrow is later used by `print(r)` in a later iteration of the loop

### Requirement: References Carry Their Borrows
A reference copied into another local, wrapped as a `(&T)?`, borrowed itself, reborrowed, or borrowed through for one of its referent's fields MUST keep the borrows it holds live for as long as the new reference is live. A value written through a reference MUST become held by what the reference points at. Re-pointing a `var` reference MUST end the borrows it held, from that point on.

#### Scenario: A copied reference keeps the borrow
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { var x = 1; val r = &x; val r2 = r; x = 2; print(r2) }` is checked
- **THEN** the borrow checker emits `E5004` at `x` in `x = 2`

#### Scenario: Re-pointing a reference variable ends its old borrow
- **WHEN** `fun print(n: &Int) {}` is declared, and `fun f() { var a = 1; var b = 2; var r: &Int = &a; r = &b; a = 3; print(r) }` and `fun g() { var a = 1; var b = 2; var r: &Int = &a; r = &b; b = 3; print(r) }` are checked
- **THEN** the borrow checker accepts `f` and emits `E5004` at `b` in `b = 3` of `g`

#### Scenario: Re-pointing a mutable reference at the value it borrowed
- **WHEN** `fun bump(r: &mut Int) {}` is declared, and `fun f() { var x = 1; var r: &mut Int = &mut x; bump(r); r = &mut x; bump(r) }` and `fun g(c: Bool) { var x = 1; var r: &mut Int = &mut x; while (c) { bump(r); r = &mut x } }` are checked
- **THEN** the file passes `prism check`, because the new borrow of `x` replaces the old one, which `r` no longer holds once it is assigned

#### Scenario: A reborrow keeps its source borrowed
- **WHEN** `struct User { var name: String }` is declared and a body runs `var u = User("a"); val m = &mut u; val n = &mut m.name; u.name = "b"; *n = "c"`
- **THEN** the borrow checker emits `E5004` at `u.name`, because `n` borrows through `m`, which borrows `u`

#### Scenario: References of references
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { var x = 1; var r = &x; val rr = &r; x = 2; print(rr) }` is checked
- **THEN** the borrow checker emits `E5004` at `x` in `x = 2`

#### Scenario: Writing a borrow through a reference
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { var a = 1; var b = 2; var r: &Int = &b; val rr = &mut r; *rr = &a; a = 3; print(r) }` is checked
- **THEN** the borrow checker emits `E5004` at `a` in `a = 3`, because `r` now refers to `a`

#### Scenario: An optional borrow keeps the borrow
- **WHEN** `struct Profile { var nick: String? }` and `fun show(n: (&String)?) {}` are declared and a body runs `var p = Profile(null); val a: (&String)? = &p.nick; p.nick = null; show(a)`
- **THEN** the borrow checker emits `E5004` at `p.nick` in `p.nick = null`

### Requirement: Values Hold Only the Borrows Their Type Can Reach
A value MUST hold a borrow of a reference itself only when its type can refer to a reference (`&&T`, `&(&T)?`) or is a function type. A reference copied out through another, a call's result and a value written through a `&mut &T` MUST hold the borrows of what the reference refers to, not a borrow of the reference.

#### Scenario: A reference read through another holds what it refers to
- **WHEN** `fun print(n: &Int) {}` is declared, and `fun f() { var a = 1; var b = 2; var r: &Int = &a; val rr = &r; val c: &Int = rr; r = &b; print(c) }`, `fun g() { var a = 1; var b = 2; var r: &Int = &a; val h = { r }; val q = h(); r = &b; print(q) }` and `fun k() { var a = 1; var r: &Int = &a; val rr = &r; val c: &Int = rr; a = 3; print(c) }` are checked
- **THEN** the borrow checker accepts `f` and `g`, where `c` and `q` are copies of `r`'s value that do not borrow `r`, and emits `E5004` at `a` in `a = 3` of `k`, because `c` still refers to `a`

### Requirement: Calls and Closures Write Borrows Through What They Are Given
A call MUST be assumed to write the borrows of its other arguments through each `&mut R` or `(&mut R)?` argument whose `R` holds borrows, through each function-typed argument and through a called function value, and a closure the borrows of its other captures through each variable it captures mutably. What these point at MUST hold those borrows afterwards, however the reference reached them.

#### Scenario: Writing a borrow through a closure
- **WHEN** `fun print(n: &Int) {}`, `fun run(f: () -> Int): Int = f()` and `fun apply(g: (&Int) -> Int, v: &Int): Int = g(v)` are declared, and `fun f(c: Bool) { var b = 2; var r: &Int = &b; val rr = &mut r; if (c) { val a = 1; run { *rr = &a; 1 } }; print(r) }` and `fun g(c: Bool) { var b = 2; var r: &Int = &b; if (c) { val a = 1; apply({ v -> r = v; 1 }, &a) }; print(r) }` are checked
- **THEN** the borrow checker emits `E5006` for `a` in `f`, because the closure may write a borrow of `a` into `r` through `rr`, and in `g`, because `apply` may pass `&a` to the closure that writes `r`

#### Scenario: Writing a borrow through a mutable reference argument
- **WHEN** `fun print(n: &Int) {}`, `fun h(s: &mut &Int, v: &Int) {}` and `fun set(s: (&mut &Int)?, v: &Int) {}` are declared, and `fun g(s: &mut &Int) { val n = 1; val t = s; h(t, &n) }` and `fun f(c: Bool) { var b = 2; var r: &Int = &b; if (c) { val a = 1; set(&mut r, &a) }; print(r) }` are checked
- **THEN** the borrow checker emits `E5006` for `n` in `g`, because `h` may write `&n` into the caller's reference that `s` points at, and for `a` in `f`, because `set` may write `&a` into `r`

### Requirement: Disjoint Fields Are Borrowed Independently
Borrows of different fields of one value MUST NOT conflict, whether the value is a local or is reached through a reference. A borrow of the value itself, or of a field, MUST conflict with a borrow of any place inside it.

#### Scenario: Disjoint field borrows
- **WHEN** `struct User { var name: String }`, `struct Order { var user: User, var email: String }` and `fun touch(a: &mut User, b: &mut String) {}` are declared, and `fun f(o: &mut Order) { touch(&mut o.user, &mut o.email) }` and `fun g(o: Order) { var p = o; val a = &mut p.user; val b = &mut p.email; touch(a, b) }` are checked
- **THEN** the file passes `prism check`

#### Scenario: A borrow of the whole value conflicts with a field borrow
- **WHEN** `struct User { var name: String }`, `struct Order { var user: User, var email: String }` and `fun touch(a: &mut User, b: &mut String) {}` are declared and a body runs `var p = Order(User("a"), "b"); val a = &mut p.user; val b = &p; touch(a, &mut p.email)`
- **THEN** the borrow checker emits `E5003` at `&p`, and not at `&mut p.email`

### Requirement: Mutable Receivers Are Borrowed After the Arguments
The mutable borrow of a `mut fun` call's receiver MUST start when the call is made, after its arguments are evaluated, so arguments that read the receiver and end before the call MUST NOT conflict with it. A borrow that an argument still holds when the call is made MUST conflict with it.

#### Scenario: Two-phase receiver borrows
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}`, `mut fun Stack.pushRef(n: &Int) {}` and `fun Stack.size(): Int = count` are declared, and a body runs `var v = Stack(0); v.push(v.size()); v.push(v.count)`, or `var w = Stack(0); w.pushRef(&w.count)`
- **THEN** the borrow checker accepts `v.push(v.size())` and `v.push(v.count)`, and emits `E5003` at the receiver `w` of `pushRef`, because `&w.count` is still held by the call

### Requirement: Safe-Call Receivers Are Borrowed at Their Test
A receiver that a `?.` test reaches (the payload of a nullable), shared or mutable, MUST be borrowed right after its test, before the arguments, so an argument that changes or mutably borrows the tested value MUST conflict with that borrow, as MUST one that reads it while the receiver is borrowed mutably.

#### Scenario: A safe-call receiver is borrowed at its test
- **WHEN** `struct Stack { var count: Int }`, `mut fun Stack.push(n: Int) {}`, `fun Stack.show(n: Int) {}` and `fun clear(s: &mut Stack?): Int { *s = null; return 1 }` are declared, and a body runs `var n: Stack? = Stack(0); n?.push(clear(&mut n))`, or `var m: Stack? = Stack(0); m?.show(clear(&mut m))`
- **THEN** the borrow checker emits `E5003` at `&mut n` and at `&mut m`, because the receiver is borrowed from the test of its tag, which `clear` could otherwise make `null` before the call

### Requirement: Calls Returning References Keep Their Inputs Borrowed
The result of a call whose type contains a reference outside a function type MUST hold the borrows passed to the inputs its elision rule ties it to (see "The Elision Rules Tie a Returned Borrow to Inputs"), each in its own mode, so a `&mut` argument it is tied to MUST stay frozen while the result is live: for a method with a borrowed receiver, the receiver's borrows only; for any other function, those of every input borrow. Function values and defaults follow their own requirements below.

#### Scenario: A returned borrow keeps every input borrowed
- **WHEN** `fun first(a: &String, b: &String): &String = a`, `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared, and `fun f(x: String, y: String) { val r = first(&x, &y); take(y); greet(r) }` and `fun g(x: String, y: String) { val r = first(&x, &y); greet(r); take(y) }` are checked
- **THEN** the borrow checker emits `E5005` at `y` in `take(y)` of `f`, and accepts `g`

#### Scenario: A mutable input stays frozen while the result is live
- **WHEN** `fun pick(a: &mut Int): &Int = a` and `fun print(n: &Int) {}` are declared and `fun f() { var x = 1; val r = pick(&mut x); print(x); print(r) }` is checked
- **THEN** the borrow checker emits `E5003` at `x` in `print(x)`

#### Scenario: A method result keeps its receiver borrowed
- **WHEN** `struct Team { var lead: String }`, `fun Team.captain(): &String = &lead` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a"); val c = t.captain(); t.lead = "b"; greet(c)`
- **THEN** the borrow checker emits `E5004` at `t.lead`

#### Scenario: A method result does not keep its other arguments borrowed
- **WHEN** `struct Team { var lead: String }`, `mut fun Team.leader(o: &String): &mut String = &mut lead` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a"); var s = "x"; val l = t.leader(&s); s = "y"; greet(l)`
- **THEN** the file passes `prism check`, because the result of `leader` is tied to its receiver only

### Requirement: The Elision Rules Tie a Returned Borrow to Inputs
Which inputs a returned borrow (a result whose type contains a reference outside a function type) comes from MUST follow from the signature alone, without lifetime annotations. Input borrows are the parameters whose type contains a reference outside a function type and the receiver of a non-`consuming` method. Rules 1 (one input borrow) and 3 (several) tie the result to every input borrow, rule 2 (a borrowed receiver) to the receiver only, and rule 4 (none) to nothing.

#### Scenario: Rule 1 - Single input borrow parameter
- **WHEN** `struct Team { var lead: String; var nick: String? }`, `fun nick(t: &Team): (&String)? = &t.nick`, `fun lead(t: &Team): &String = &t.lead` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a", null); val l = lead(&t); t.lead = "b"; greet(l)`
- **THEN** the borrow checker accepts `nick` and `lead`, and emits `E5004` at `t.lead` in the assignment, because the result of `lead` is tied to its argument

#### Scenario: Rule 3 - Multiple input borrow parameters
- **WHEN** `fun longest(a: &String, b: &String, c: Bool): &String = if (c) a else b`, `fun pick(a: &mut Int, b: &Int): &Int = b` and `fun print(n: &Int) {}` are declared and `fun f() { var x = 1; val y = 2; val r = pick(&mut x, &y); print(x); print(r) }` is checked
- **THEN** the borrow checker accepts `longest` and `pick`, and emits `E5003` at `x` in `print(x)`, because the result of `pick` borrows from both inputs, each in its own mode, whatever its body returns

#### Scenario: The signature decides, not the body
- **WHEN** `struct User { val name: String }`, `struct Team { var lead: String }`, `val DEFAULT = User("guest")`, `fun Team.fallback(): &User = &DEFAULT` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a"); val f = t.fallback(); t.lead = "b"; greet(f.name)`
- **THEN** the borrow checker emits `E5004` at `t.lead`, because rule 2 ties the result of `fallback` to its receiver, although its body returns a borrow of a top-level `val`

#### Scenario: Elision across files uses the imported signature
- **WHEN** `lib/team.prism` declares `public struct Team { var lead: String }`, `public fun Team.captain(o: &String): &String = &lead` and `public fun first(a: &String, b: &String): &String = a`, and `app/main.prism` imports `/lib/team` and declares `fun greet(s: &String): Int = 1` and `fun take(s: String) {}`, and either `fun f(t: Team, s: String): Int { var u = t; var x = s; val c = u.captain(&x); x = "y"; return greet(c) }` or `fun g(a: String, b: String): Int { val r = first(&a, &b); take(b); return greet(r) }`
- **THEN** `prism check app/main.prism` accepts `f`, whose call result is tied to the receiver only by the imported signature of `captain`, and emits `E5005` at `b` in `take(b)` of `g`, whose call result is tied to both arguments of `first`

### Requirement: Returned Borrows Derive Only From Tied Inputs
A body MUST return only borrows of top-level `val`s, which are valid everywhere, or borrows derived from the inputs its elision rule ties the result to. The borrow checker MUST reject a returned borrow of a local, a temporary or a by-value parameter with `DanglingBorrowReturn` (`E5008`), and one derived from a parameter the result is not tied to with `UntiedBorrowReturn` (`E5009`).

#### Scenario: Rule 2 - Method receiver borrow
- **WHEN** `struct Team { var lead: String }`, `fun greet(s: &String): Int = 1` and `fun Team.captain(o: &String): &String = &lead` are declared and `fun f(t: Team, s: String): Int { var u = t; var x = s; val c = u.captain(&x); x = "y"; return greet(c) }` is checked, or `fun Team.other(o: &String): &String = o` is checked
- **THEN** the borrow checker accepts `f`, because the result of `captain` is tied to the receiver `u` only, and emits `E5009` at `o` in the body of `other`, which returns a borrow derived from a parameter other than its receiver

#### Scenario: Rule 4 - Zero input borrows returning a borrow
- **WHEN** `fun f(): &Int { val n = 1; return &n }` and `fun g(s: String): &String = &s` are checked
- **THEN** the borrow checker emits `E5008` at `&n` in `f`, a borrow of the local `n`, and at `&s` in `g`, a borrow of the by-value parameter `s`

#### Scenario: Returning a borrow of a local under any rule
- **WHEN** `struct Team { var lead: String }` and `fun make(): String = "x"` are declared, and `fun f(t: &Team): &String { val s = "x"; return &s }`, `fun g(t: &Team): &String = &make()` and `fun Team.h(): &String { val c = lead.clone(); return &c }` are checked
- **THEN** the borrow checker emits `E5008` for each: at `&s` for the local `s`, at `&make()` for a temporary value, and at `&c` for the local `c`

#### Scenario: Borrow of a top-level val
- **WHEN** `struct User { val name: String }`, `val DEFAULT = User("guest")`, `fun greet(s: &String): Int = 1`, `fun guest(): &User = &DEFAULT` and `fun orGuest(u: &User, c: Bool): &User = if (c) u else &DEFAULT` are declared and `fun f(): Int { val g = guest(); return greet(g.name) }` is checked
- **THEN** the file passes `prism check`: a borrow of a top-level `val` may be returned under any rule, including from `guest`, which has no input borrow, and the result of `guest` keeps nothing borrowed

#### Scenario: A top-level value cannot borrow a temporary
- **WHEN** `fun make(): String = "x"`, `val LIMIT = 10`, `val REF: &Int = &LIMIT`, `val R: &Int = &1` and `val S: &String = &make()` are checked
- **THEN** the borrow checker accepts `REF` and emits `E5008` at `&1` and at `&make()`, borrows of temporaries that end with the initializer

#### Scenario: A borrow read through a reference or a closure
- **WHEN** `struct Team { val lead: String }` and `fun lead(t: &Team): &String = &t.lead` are declared, and `fun f(a: &Int): &Int { val rr = &a; val x: &Int = rr; return x }`, `fun g(t: &Team): &String { val h = { lead(t) }; return h() }` and `fun Team.name(): &String { val h = { &lead }; return h() }` are checked
- **THEN** the file passes `prism check`: each returned borrow derives from the input its rule ties the result to, not from the local reference or the closure's borrow of the parameter it was read through

### Requirement: Function Types Elide by the Same Rules
A function type MUST elide by rules 1, 3 and 4: a call of a function value MUST tie its result to every input-borrow argument, or, when its type has no input borrow, to the function value's own borrows. So a closure MUST return only borrows of top-level `val`s or derived from its reference parameters (`E5009` otherwise), or, when its type has no input borrow, also of what it captures; a borrow of its own local or temporary is `E5008`.

#### Scenario: A closure returning a borrow
- **WHEN** `fun app(g: (&Int) -> &Int): Int = 1` is declared and `fun f(): Int = app { k -> k }` and `fun h(): Int = app { k -> val m = 2; &m }` are checked
- **THEN** the borrow checker accepts `f` and emits `E5008` at `&m` in the closure of `h`

#### Scenario: A closure may return a borrow of what it captures
- **WHEN** `fun greet(s: &String): Int = 1` is declared, and `fun f(s: String): Int { var t = s; val g = { &t }; val r = g(); t = "b"; return greet(r) }` and `fun h(s: String): Int { var t = s; val g = { &t }; val r = g(); greet(r); t = "b"; return 1 }` are checked
- **THEN** the borrow checker accepts both closures, emits `E5004` at `t` in `t = "b"` of `f`, because `r` keeps the closure's borrow of `t`, and accepts `h`

#### Scenario: A closure taking a borrowed parameter cannot return its captures
- **WHEN** `fun app(g: (&Int) -> &Int): Int = 1` is declared and `fun f(): Int { val n = 1; return app { k -> &n } }` is checked
- **THEN** the borrow checker emits `E5009` at `&n` in the closure, naming the captured `n`, because the result of a closure of type `(&Int) -> &Int` is tied to its parameter `k` only

#### Scenario: Returning a borrow obtained from a function-typed parameter
- **WHEN** `fun call(h: () -> &Int): &Int = h()` and `fun apply(g: (&Int) -> &Int, v: &Int): &Int = g(v)` are checked
- **THEN** the borrow checker emits `E5009` for `call`, naming the function-typed parameter `h`, since a caller does not keep what the closure passed for it borrows, and accepts `apply`, since the result of `g(v)` is tied to `v` by `g`'s type (elision rule 1) and `v` is `apply`'s single input borrow

### Requirement: Default Values Borrow Only Through Shared References
A reference-typed parameter's default MUST return only a borrow of the temporary in the caller's slot, of a top-level `val`, or derived from what an earlier shared reference parameter (`&T`, `(&T)?`) or a `fun` receiver refers to. A default borrowing an earlier by-value parameter, or deriving a borrow from an earlier `&mut` or function-typed parameter or a `mut fun` receiver, which the call passes beside it, MUST be rejected at its declaration with `E5009`.

#### Scenario: A default borrowing an earlier by-value parameter
- **WHEN** `fun m(a: String, b: &String = a): Int = 1` and `fun k(a: Int, b: &Int = a): Int = 1` are checked
- **THEN** the borrow checker emits `E5009` at `a` in each default value, because the call takes `a` by value

#### Scenario: A default borrowing a mutable receiver
- **WHEN** `struct User { var name: String }` is declared, and `mut fun User.rename(n: &String = name) {}`, `fun f(m: &mut User, n: &String = m.name): Int = 1` and `fun g() { var u = User("a"); u.rename() }` are checked
- **THEN** the borrow checker emits `E5009` at `name` in the default of `rename` and at `m.name` in the default of `f`, and nothing at the call `u.rename()`

#### Scenario: A default reading through a shared reference keeps it borrowed
- **WHEN** `struct User { var name: String }`, `fun User.hi(n: &String = name): &String = &name`, `fun hello(u: &User, n: &String = u.name): &String = n`, `fun greet(s: &String): Int = 1` and `fun pair(s: &String, u: &mut User): Int = 1` are declared, and `fun f(): Int { var u = User("a"); val h = u.hi(); greet(h); return greet(hello(&u)) }`, `fun g(): Int { var u = User("a"); return pair(hello(&u), &mut u) }` and `fun k(): Int { var u = User("a"); val h = hello(&u); return greet(h) }` are checked
- **THEN** the borrow checker accepts both defaults and `f`, where `h` keeps only the receiver `u` borrowed; emits `E5003` at `&mut u` in `g`, because the result of `hello` borrows `u` through its argument and its default; and emits `E5006` at `hello(&u)` in `k`, because that result is also tied to the default's slot, which ends with the caller's statement

### Requirement: Default Value Results Keep Their Shared Inputs Borrowed
The result of a default value MUST hold the borrows of its slot and what the earlier shared reference arguments, and a `fun` receiver, borrow, and nothing else: not the default's own borrows of the earlier arguments (see "Default Values Borrow Only Through Shared References").

#### Scenario: An omitted reference default after a by-value parameter
- **WHEN** `fun greet(a: String, n: &String = "anon"): Int = 1` is declared and `fun f(s: String): Int = greet(s)` is checked
- **THEN** the file passes `prism check`

### Requirement: Borrows Stored Through Inputs Outlive the Call
A body MAY store a borrow derived from its inputs through a `&mut R` parameter whose `R` holds borrows, and a closure into a variable it captures, since callers keep what such a parameter or variable points at borrowed by every other operand (see "Calls and Closures Write Borrows Through What They Are Given"). A by-value parameter MUST end when its body returns, so storing a borrow of a local, a temporary or a by-value parameter there MUST be rejected with `DroppedWhileBorrowed` (`E5006`).

#### Scenario: Storing an input borrow through another input
- **WHEN** `fun store(slot: &mut &Int, v: &Int) { *slot = v }` is checked
- **THEN** the file passes `prism check`

#### Scenario: Storing a borrow of a by-value parameter
- **WHEN** `fun run2(f: (Int) -> Int): Int = f(1)` and `fun print(n: &Int) {}` are declared, and `fun keep(slot: &mut &Int, v: Int) { *slot = &v }` and `fun g() { var a = 1; var r: &Int = &a; run2 { x -> r = &x; 1 }; print(r) }` are checked
- **THEN** the borrow checker emits `E5006` at `&v`, which `keep`'s caller would see through `slot` after `v` ends, and at `&x`, which `r` would keep after the closure returns

#### Scenario: Storing a copy of an input borrow through another input
- **WHEN** `fun run(f: () -> Int): Int = f()` and `fun set(s: &mut &Int, v: &&Int) {}` are declared, and `fun keep(slot: &mut &Int, v: &Int) { run { *slot = v; 1 } }` and `fun put(slot: &mut &Int, v: &Int) { set(slot, &v) }` are checked
- **THEN** the file passes `prism check`, because what is written through `slot` is a copy of `v`, never a borrow of the parameter `v`, which ends when the function returns

### Requirement: Borrowed Values Outlive Their Borrows
A local or temporary MUST NOT be dropped, or end its storage, while a borrow of it is live. Such a drop MUST be rejected by the borrow checker with `DroppedWhileBorrowed` (`E5006`), pointing at the borrow, with a related location where the value ends.

#### Scenario: Borrow of a local that goes out of scope
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }` is checked
- **THEN** the borrow checker emits `E5006` at `&n`, saying that `n` does not live long enough, with a related location at the closing brace of the `if` branch

#### Scenario: Borrow of a temporary that is dropped
- **WHEN** `fun make(): String = "x"`, `fun first(a: &String, b: &String): &String = a` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String) { var r: &String = &s; r = &make(); greet(r) }` and `fun g(s: String) { val r = first(&make(), &s); greet(r) }` are checked
- **THEN** the borrow checker emits `E5006` at `&make()` in `f` and in `g`, saying that a temporary value is dropped while borrowed, with help suggesting to bind the value to a local first

#### Scenario: Borrowed temporaries that are extended
- **WHEN** `fun make(): String = "x"` and `fun greet(s: &String): Int = 1` are declared, and `fun f() { val r = &make(); greet(r) }`, `fun g(c: Bool, s: String) { val r = if (c) &make() else &s; greet(r) }` and `fun h(n: String?): Int = greet(&n ?: &"anon")` are checked
- **THEN** the file passes `prism check`, because each temporary lives as long as `r`, or until the end of the statement that borrows it

#### Scenario: A borrow of a loop's local kept for the next iteration
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f(c: Bool) { var a = 1; var r: &Int = &a; while (c) { val n = 1; print(r); r = &n } }` is checked
- **THEN** the borrow checker emits `E5006` at `&n`, with the later use `print(r)` labelled as in a later iteration of the loop, and nothing for the assignment of the next iteration's `n`

#### Scenario: A closure stores a borrow of its own local in a capture
- **WHEN** `fun print(n: &Int) {}` and `fun run(f: () -> Int): Int = f()` are declared and `fun f() { var a = 1; var r: &Int = &a; run { val n = 2; r = &n; 1 }; print(r) }` is checked
- **THEN** the borrow checker emits `E5006` at `&n`, because the closure's local `n` ends while `r` refers to it

### Requirement: Closures Capture by Inferred Borrow
A closure MUST borrow each variable it captures: mutably when its body assigns the variable or one of its fields, writes through it, borrows it or a part of it mutably, or contains a closure that captures it mutably, and shared otherwise. The borrows MUST start where the closure is created and last while the closure value is live. Conflicting uses of a captured variable meanwhile MUST be rejected as for any borrow.

#### Scenario: Closures capture by inferred borrow
- **WHEN** `struct User { var name: String }`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared, and a body declares `var u = User("a")` and runs `val f = { u.name = "z"; 1 }; greet(u.name); run(f)`, or `val g = { greet(u.name) }; greet(u.name); run(g)`, or `val h = { greet(u.name) }; u.name = "b"; run(h)`, or `val k = { u.name = "z"; 1 }; run(k); u.name = "b"`
- **THEN** the borrow checker emits `E5003` at `u.name` in `greet(u.name)` after `f`, accepts `g`, emits `E5004` at `u.name` in the assignment after `h`, and accepts `k`, whose borrow ends when `run(k)` returns

#### Scenario: Moving a captured variable while the closure is live
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(s: String) { val g = { greet(s) }; take(s); run(g) }` is checked
- **THEN** the borrow checker emits `E5005` at `s` in `take(s)`

#### Scenario: A nested closure makes the capture mutable
- **WHEN** `fun run(f: () -> Int): Int = f()` and `fun print(n: &Int) {}` are declared and a body runs `var x = 1; val f = { run { x = 2; 1 } }; print(x); run(f)`
- **THEN** the borrow checker emits `E5003` at `x` in `print(x)`, because the outer closure borrows `x` mutably for the inner one

### Requirement: Capturing Closures Do Not Escape
A function value that may be a closure with captures, or a function-typed parameter, MUST NOT escape: being returned from a function, closure or default value, initializing a top-level `val`, being stored in a struct field, an enum payload or a box, being written through a reference, or being assigned to a captured variable MUST be rejected by the borrow checker with `EscapingClosure` (`E5007`). A closure without captures MAY escape. Passing any closure to a function-typed parameter is not one.

#### Scenario: Capturing closures do not escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): () -> Int = { greet(s) }`, `fun g(s: String): Button = Button({ greet(s) })`, `fun h(slot: &mut (() -> Int), s: String) { *slot = { greet(s) } }`, `fun k(p: () -> Int): () -> Int = p` and `fun m(a: Int, p: () -> Int = { a }): Int = p()` are checked
- **THEN** the borrow checker emits `E5007` for each: for `f` saying that a closure that captures `s` cannot escape and labelling where it is returned, for `g`, `h` and `m` likewise, and for `k` saying that the function-typed parameter `p` cannot escape

#### Scenario: A closure without captures may escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun run(f: () -> Int): Int = f()` are declared, and `fun f(): () -> Int = { 1 }`, `fun g(): Button = Button({ 2 })` and `fun h(s: String): Int = run { s.clone(); 3 }` are checked
- **THEN** the file passes `prism check`

#### Scenario: Boxing a capturing closure
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `fun f(s: String): Int { val h = Heap({ greet(s) }); return 1 }` and `fun g(): Heap<() -> Int> = Heap({ 2 })` are checked
- **THEN** the borrow checker emits `E5007` for `f`, labelling the closure "stored in a box here", and accepts `g`

### Requirement: Capturing Closures Are Not Passed to Generic Parameters
A function value that may be a closure with captures, or a function-typed parameter, MUST NOT be passed where the callee's declared parameter type has a type parameter outside a function type, since the generic body may keep it: the borrow checker MUST reject it with `E5007`, labelling it as passed to a generic parameter. Passing it to a parameter of function type, such as `() -> T`, MUST be allowed.

#### Scenario: A capturing closure passed to a generic parameter
- **WHEN** `fun <T> keep(x: T): T = x`, `fun <T> remember(calc: () -> T): T = calc()` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): Int { val g = keep({ greet(s) }); return g() }`, `fun k(p: () -> Int): Int = keep(p)()`, `fun h(s: String): Int = remember { greet(s) }` and `fun n(): Int = keep({ 1 })()` are checked
- **THEN** the borrow checker emits `E5007` for `f`, labelling the closure "passed to a generic parameter here", and for `k`, and accepts `h`, whose closure is passed to a function-typed parameter, and `n`, whose closure captures nothing

### Requirement: Borrow Conflict Diagnostics Show the Borrow and Its Later Use
A diagnostic of `E5003`, `E5004` or `E5005` MUST point at the conflicting access and name the place, and MUST list as related locations where the conflicting borrow was created and where it is used later, with ", in later iteration of loop" when that use is reached only through a loop's back edge. A borrow made by a closure MUST also label the use inside the closure that decided how it is captured.

#### Scenario: Conflict points at the borrow and its later use
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f() { var x = 1; val r = &x; x = 2; print(r) }`
- **THEN** it reports `error[E5004]: cannot assign to 'x' because it is borrowed` at `x` in `x = 2`, labelled "'x' is assigned here while borrowed", with a related location at `&x` labelled "'x' is borrowed here" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Closure borrow labels
- **WHEN** `struct User { var name: String }`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `prism check` runs on `fun f() { var u = User("a"); val g = { u.name = "z"; 1 }; greet(u.name); run(g) }`
- **THEN** it reports `error[E5003]: cannot borrow 'u.name' as shared because 'u' is borrowed as mutable` at `u.name` in `greet(u.name)`, labelled "shared borrow occurs here", with related locations at the closure labelled "closure borrows 'u' mutably here", at `u.name` inside the closure labelled "borrow occurs due to this use of 'u' in the closure", and at `g` in `run(g)` labelled "mutable borrow later used here"

### Requirement: Lifetime and Escape Diagnostics Show Where the Value Ends
An `E5006` diagnostic MUST point at the borrow, with related locations where the borrowed value ends and where the borrow is used later. A parameter that ends when its body returns MUST be labelled at its declaration, and a later use by the caller, through a `&mut R` parameter, at that parameter. An `E5007` diagnostic MUST point at where the value escapes, with a related location at the capture or the parameter.

#### Scenario: Dropped value labels
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }`
- **THEN** it reports `error[E5006]: 'n' does not live long enough` at `&n` labelled "borrowed value does not live long enough", with a related location at the `}` closing the branch labelled "'n' dropped here while still borrowed" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Dropped parameter labels
- **WHEN** `prism check` runs on `fun keep(slot: &mut &String, v: String) { *slot = &v }`
- **THEN** it reports `error[E5006]: 'v' does not live long enough` at `&v` labelled "borrowed value does not live long enough", with a related location at the parameter `v` labelled "'v' is dropped when the function returns, while still borrowed" and one at the parameter `slot` labelled "borrow later used by the caller through 'slot'"

#### Scenario: Escaping closure labels
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `prism check` runs on `fun f(s: String): () -> Int = { greet(s) }`
- **THEN** it reports `error[E5007]: a closure that captures 's' cannot escape` at the closure labelled "returned here", with a related location at `s` inside the closure labelled "'s' is captured here", a note that a closure borrows what it captures, and help suggesting to pass `s` to the closure as a parameter

### Requirement: Returned Borrow Diagnostics Name the Source
An `E5008` diagnostic MUST point at where the borrow is returned, name the local, temporary or parameter it borrows, and label where it is borrowed when that is elsewhere; for a function with no input borrow it MUST note that only borrows of top-level values can be returned. An `E5009` diagnostic MUST point at where the borrow is returned, name the parameter it derives from with a related location at that parameter, and note the rule that does not tie the result to it.

#### Scenario: Dangling returned borrow labels
- **WHEN** `prism check` runs on `fun f(): &Int { val n = 1; val r = &n; return r }`
- **THEN** it reports `error[E5008]: cannot return a borrow of local 'n'` at `r` in `return r` labelled "returned here", with a related location at `&n` labelled "'n' is borrowed here", and a note that `f` takes no borrowed input, so it can only return borrows of top-level values (elision rule 4)

#### Scenario: Untied returned borrow labels
- **WHEN** `struct Team { var lead: String }` is declared and `prism check` runs on `fun Team.other(o: &String): &String = o`
- **THEN** it reports `error[E5009]: method 'other' cannot return a borrow derived from parameter 'o'` at `o` in the body labelled "returned here", with a related location at the parameter `o` labelled "'o' is a parameter here", and a note that a method's returned borrow is tied to its receiver only (elision rule 2)

### Requirement: Compound Assignments Read and Write Their Target
The borrow checker MUST treat a compound assignment as a read of its target followed by a write of it. A live borrow of an overlapping place, shared or mutable, MUST be reported once, as `E5004` at the target. A closure whose body compound-assigns a captured variable or one of its fields MUST capture it mutably. A target that may have been moved MUST be rejected with `E5001`.

#### Scenario: Compound assignment while borrowed
- **WHEN** `fun print(n: &Int) {}` is declared and a body runs `var x = 1; val r = &x; x += 1; print(r)`, or `var y = 1; val m = &mut y; y += 1; print(m)`, or `var z = 1; val k = &z; print(k); z += 1`
- **THEN** the borrow checker emits `E5004` at `x` in `x += 1`, a single `E5004` at `y` in `y += 1`, not also an `E5003` for its read, and accepts the last, whose borrow ends at `print(k)`

#### Scenario: A closure compound-assigning a capture borrows it mutably
- **WHEN** `fun run(f: () -> Int): Int = f()` and `fun print(n: &Int) {}` are declared and a body runs `var count = 0; val f = { count += 1; count }; print(count); run(f)`, or `var total = 0; run { total += 1; total }; print(total)`
- **THEN** the borrow checker emits `E5003` at `count` in `print(count)`, and accepts the second, whose closure's borrow ends when `run` returns

#### Scenario: Compound assignment to a moved string
- **WHEN** `fun take(s: String) {}` is declared and `fun f() { var s = "a"; take(s); s += "b" }` is checked
- **THEN** the borrow checker emits `E5001` at `s` in `s += "b"`, with a related location at the `s` passed to `take`

### Requirement: Borrowing Bindings Borrow the Subject's Parts
A binding of a borrowed subject's part MUST be a borrow of that part, carrying the subject's loan. It is shared for a `&E` subject and mutable for a `&mut E` one. Like any borrow, it lasts until its last use, so assigning, moving or mutably borrowing the subject while it is live MUST be rejected as for any other borrow (`E5003` to `E5005`).

#### Scenario: Assigning the subject while a binding is live
- **WHEN** `enum Msg { Text(String), Empty }` and `fun len(s: &String): Int = 1` are declared and `fun f(m: Msg): Int { var v = m; return when (&v) { Msg.Text(val s) -> { v = Msg.Empty; len(s) }; Msg.Empty -> 0 } }` is checked
- **THEN** the borrow checker emits `E5004` at the assignment to `v`, labelling the borrow `&v` and the later use of `s`

#### Scenario: A binding's borrow ends at its last use
- **WHEN** with that `Msg` and `len`, `fun f(m: Msg): Int { var v = m; return when (&v) { Msg.Text(val s) -> { val n = len(s); v = Msg.Empty; n }; Msg.Empty -> 0 } }` is checked
- **THEN** the file passes the borrow checker

#### Scenario: A mutable binding is exclusive
- **WHEN** with that `Msg`, `fun count(m: &Msg): Int = 1` and `fun f(m: &mut Msg): Int = when (m) { Msg.Text(val s) -> { val n = count(m); *s = "x"; n }; Msg.Empty -> 0 }` are checked
- **THEN** the borrow checker emits `E5003` at `m` in `count(m)`, labelling the mutable borrow of the payload and its later use in `*s = "x"`

### Requirement: Disjoint Payloads Are Borrowed Independently
Borrows of different payloads of the same variant MUST NOT conflict, as borrows of different fields do not. Payloads of different variants share storage, so their places MUST overlap.

#### Scenario: Two mutable payload bindings
- **WHEN** `enum Pair { Of(String, String) }` and `fun swap(a: &mut String, b: &mut String) {}` are declared and `fun f(p: &mut Pair) { when (p) { Pair.Of(val a, val b) -> swap(a, b) } }` is checked
- **THEN** the file passes the borrow checker

### Requirement: A Box Lends Its Contents to Reference Parameters
Where a call's parameter has type `&T`, an argument of type `Heap<T>`, behind any number of boxes and references, MUST lend its contents: a place MUST be read where it lives and its contents borrowed, without being consumed, and any other argument MUST be a temporary whose contents are borrowed until the end of the statement. A `Heap<T>?` argument MUST lend its contents to a `(&T)?` parameter likewise.

#### Scenario: Passing a box where its contents are borrowed
- **WHEN** `enum Expr { Lit(Int), Neg(Heap<Expr>) }` and `fun eval(e: &Expr): Int = 0` are declared, and a body of `fun f(h: Heap<Expr>, r: &Heap<Expr>, hh: Heap<Heap<Expr>>)` runs `eval(h)`, `eval(h)`, `eval(r)`, `eval(hh)` and `eval(Heap(Expr.Lit(1)))`
- **THEN** the body typechecks successfully, and `h` is not reported as moved

#### Scenario: Passing an optional box where an optional borrow is expected
- **WHEN** `struct Tree { val value: Int, val left: Heap<Tree>? }` and `fun depth(t: (&Tree)?): Int = 0` are declared and `fun f(t: &Tree): Int = depth(t.left)` is checked
- **THEN** the file typechecks successfully, and `t.left` is not reported as moved

#### Scenario: A box is not lent to a mutable parameter implicitly
- **WHEN** `fun bump(r: &mut Int) {}` is declared and `fun f(h: Heap<Int>) { var m = h; bump(m); bump(&mut m) }` is checked
- **THEN** the type checker emits `E4004` for `bump(m)`, with help suggesting `&mut m`, and accepts `bump(&mut m)`, which borrows the contents of `m` mutably

### Requirement: Borrowed Boxes Convert to Borrowed Contents
`&Heap<T>` MUST convert implicitly to `&T`, and `&mut Heap<T>` to `&mut T` or `&T`, through any number of boxes, wherever such a type is expected, as a borrow of the box's contents. A borrowed `Heap<T>?` MUST convert to `(&T)?` likewise. A value that already fits its expected type MUST NOT be converted.

#### Scenario: Returning a borrow of a box's contents
- **WHEN** `struct Node { val value: Int, val next: Heap<Node>? }` is declared, and `fun contents(h: &Heap<Node>): &Node = h` and `fun next(n: &Node): (&Node)? = &n.next` are checked
- **THEN** both typecheck successfully

#### Scenario: A generic parameter borrows the box itself
- **WHEN** `fun <T> same(x: &T): &T = x` is declared and `fun f(h: Heap<Int>): Int { val r = same(&h); return 1 }` is checked
- **THEN** the file typechecks, and `r` has type `&Heap<Int>`

### Requirement: Borrows of a Box's Contents Borrow the Box
A box MUST own its contents: a borrow of a box's contents, or of a part of them, MUST conflict with uses of the box as a borrow of a part of an owned value does, so the box MUST NOT be assigned, moved or dropped while its contents are borrowed. A borrow of the contents of a by-value box MUST NOT be returned.

#### Scenario: Using a box while its contents are borrowed mutably
- **WHEN** `struct P { var x: Int }` is declared and `fun f(h: Heap<P>): Int { var m = h; val q = &mut m.x; val z = m.x; *q = 1; return z }` is checked
- **THEN** the borrow checker emits `E5003` at `m.x` in `val z = m.x`

#### Scenario: Replacing or moving a box while its contents are borrowed
- **WHEN** `struct P { var x: Int }` and `fun take(h: Heap<P>) {}` are declared, and `fun f(h: Heap<P>): Int { var m = h; val r = &m.x; m = Heap(P(2)); return r }` and `fun g(h: Heap<P>): Int { val r = &h.x; take(h); return r }` are checked
- **THEN** the borrow checker emits `E5004` for the assignment to `m` and `E5005` for the move of `h`

#### Scenario: Returning a borrow of a by-value box's contents
- **WHEN** `struct P { var x: Int }` is declared and `fun f(h: Heap<P>): &Int = &h.x` and `fun g(h: &Heap<P>): &Int = &h.x` are checked
- **THEN** the borrow checker emits `E5008` for `f`, a borrow of the parameter `h`, and accepts `g`, whose result borrows what `h` refers to

### Requirement: Template Parts Are Borrowed Until the Template Is Built
A template MUST read each `String` part in place, through a shared borrow that lasts until the template's result is built, and MUST NOT move it; a computed part is a temporary living until the end of the enclosing statement. A part whose evaluation needs a mutable borrow of a string that an earlier part of the same template borrows MUST be rejected as a borrow conflict (`E5003`), as for the operands of `+`.

#### Scenario: Borrowed parts stay usable
- **WHEN** `fun f(s: String, r: &mut String): String { val t = "$s and $r"; *r = t; return s }` is checked
- **THEN** the file typechecks and borrow-checks successfully

#### Scenario: A later part mutating a borrowed part rejected
- **WHEN** `fun grow(s: &mut String): Int { *s = s + "!"; return s.length }` and `fun f() { var s = "a"; val t = "$s ${grow(&mut s)}" }` are checked
- **THEN** the borrow checker emits `E5003` at `&mut s`, labelling the first part's borrow of `s` and its later use, where the template is built

### Requirement: Scalar Template Parts Are Read at Their Position
A scalar part MUST be read at its position in the template, so a later part writing the same place, through a mutable borrow, does not change the earlier part's text and is not a borrow conflict.

#### Scenario: A later part writing an earlier scalar part
- **WHEN** `fun bump(r: &mut Int): Int { *r = r + 1; return r }` and `fun f(): String { var n = 1; return "$n ${bump(&mut n)} $n" }` are checked
- **THEN** the file typechecks and borrow-checks successfully, the first part reading `n` before `bump` writes it

## Design Rationale
Decisions from `reference-access` (agreed with the user in the borrow-checker roadmap exploration and while proposing the change, 2026-10-17):

- **Shared borrows are implicit at argument sites; `&mut` stays explicit** (user). A move-only `String` makes `&String` parameters common, and an explicit `&` on a read adds noise without adding safety, while mutation stays visible at the call. Default values are evaluated as the call's argument, so they borrow like one. Outside arguments, `&` is written. Rejected: explicit `&` everywhere (noise), and Swift/Hylo/Mojo-style parameter conventions, where `&T` is not a type, because they would reshape the elision rules the invariants commit to.
- **A borrow points at one place or one temporary** (user). A place (a variable, a `.` field path from any root, an index) is borrowed where it lives. Anything else, including `if`, `when`, blocks, `?:` and safe navigation, is evaluated into a consumed temporary that lives until the end of the statement. So `greet(if (c) u.name else v.name)` moves the fields (`E4017`) and `greet(&u.nick ?: &"anon")` needs `&` on each side. Distributing a borrow to the leaves of branches was rejected: one rule matches Rust, and the MIR lowers each borrow to one location. It remains a possible relaxation.
- **Arguments to reference parameters are inferred, not checked.** Such an argument may already be a reference (`greet(longest(&a, &b))`) or a value to borrow, which only its type tells apart. Nothing that needs an expected type (a closure, `null`) is ever borrowed.
- **Implicit copy-out of `Copy` values** (user). Where a `T` is expected, reference layers are peeled until the type fits. A `Copy` value is copied bitwise, and a non-`Copy` one is a move out of the borrow (`E4017`, see `ownership`). Copy-out also works through one `?` (`(&T)?` as a `T?`), since `T?` is then `Copy`. It is not applied inside `if`/`?:` joins, where a join's type would then depend on which side is a reference.
- **Prefix `*` writes through a reference, and only there** (user). Reads have one spelling, the implicit one. Every layer written through must be `&mut` (`E4003`), and `*` on a non-reference is `NotAReference` (`E4019`). `*` as a general place operator (`val x = *r`, `&mut *r`) was rejected, at the cost of having no syntax that reborrows a `&mut` into a local. Transparent C++-style references were rejected because they cannot be re-pointed, and a type-directed `r = v` because it is hard to read.
- **`var` references re-point** (user). `r = &b` assigns the reference itself. A value of the referent's type gets help pointing at `*r = v`, and a `val` reports only `E4003`.
- **Passing a `&mut` place reborrows it** (user). The binding stays usable after the call. `&mut r` on a `val r: &mut T` follows the ordinary `val` rule, because it borrows the binding: with write-through, `*(&mut r) = &mut other` would otherwise re-point a `val`.
- **Borrowed nullables are viewed as optional borrows** (user). `&(T?)` converts to `(&T)?` (and `&mut (T?)` to `(&mut T)?` or `(&T)?`), and `?.` and `?:` read through such a reference. This is a conversion, not subtyping: it reads the tag and points at the payload. The reverse is impossible, because a `(&T)?` has no `T?` to point at. Rejected: an explicit member like Rust's `Option::as_ref` (noise on a common Kotlin idiom), and leaving cloning as the only way to read a borrowed nullable.
- **References are invariant** (user). A reference points at existing storage, so it cannot change its referent's representation: `T` to `T?` adds a tag, which a value conversion can do and a pointer cannot. Only `&mut` weakening to `&` (at the top, and behind shared borrows) is allowed. Covariance would have made codegen hide a converted copy behind a "borrow".
- **One coercion step.** Implicit borrows, reborrows, the nullable view and copy-out are decided by one function at every value position after plain assignability fails, which classifies each as an adjustment for the typed tree. Folding them into assignability was rejected, because assignability is also used for joins and equality, where silent borrows and copies would be wrong. A place's move check runs after the coercion, so a borrow or copy-out moves nothing, and a value that does not fit reports only the mismatch.

Decisions from `extension-methods`:

- **Receivers are borrowed implicitly by their mode, including for `mut fun`** (user). `(&mut u).rename()` would be unreadable, and the method's declaration already says it mutates, as for Swift's `mutating` methods; this is the one exception to explicit `&mut` at call sites. The receiver of a `mut fun` must be a mutable place by the rules of `&mut`, or a temporary, matching `&mut make()`.
- **Receivers are looked through at any depth.** Every `&`, `&mut` and `Heap<T>` layer is peeled until the type matches the extension's receiver, as in Rust's auto-deref, because field access already reads through any depth. Requiring an exact match was rejected: `(*r).greet()` does not even parse, since `*` is writes-only.
- **Every method-call receiver is classified before its member is resolved.** Which member a call reaches depends on the receiver's type, which is only known once the receiver is inferred, and whether it is inferred in a consuming position is what the classification decides. So a branch over fields is a moved temporary for a field call or the built-in `clone()` as for an extension call. Resolving from a speculative inference was rejected: its errors would have to be discarded, including those of declarations inferred on demand along the way.
- **The receiver pass is decided at one point and classified** (auto-deref count, shared or mutable borrow, move or copy-out, temporary or place), so the typed tree can record it as an adjustment and MIR can emit the borrow after the arguments, which two-phase borrows need.

Decisions from `borrowck-exclusivity` (the scope agreed with the user in the borrow-checker roadmap exploration, 2026-10-17: non-lexical borrows, disjoint field paths, two-phase receiver borrows, closures capturing by inferred borrow and never escaping; the rest decided while proposing, implementing and reviewing the change and logged in `docs/decisions-for-review.md`):

- **Loans flow into locals and are live where a live local may hold them.** A loan is one borrow made in a body (written or implied, or a closure capture). A forward analysis computes which loans each local may hold at each point, a backward one which locals are used later, and an access conflicts with every live loan of an overlapping place. This is the location-sensitive formulation of Polonius on the bit-set dataflow of `borrowck-moves`, without regions or outlives constraints: non-lexical borrows come from liveness alone, re-pointing a `var` reference (`r = &a; r = &b; a = 3`) is accepted, which NLL's single region per reference rejects, and with no lifetime annotations (the zero-cost memory-safety invariant) a signature only says which inputs a returned borrow comes from. Rejected: NLL regions, full Polonius, lexical borrows (user).
- **Only references and function types carry loans.** Structs and enums cannot hold borrows (the `syntax` spec) or capturing closures, so only locals of a reference or function type, or a nullable of one, take part. An argument or capture of function type holds a "function input" fact, and an argument of type `&mut R` (or `(&mut R)?`) whose `R` carries loans a "referent input", so that an escaping function-typed parameter is caught and a write through any copy of such an argument reaches what the caller sees.
- **How loans move.** A borrow holds its loan and every loan of the borrowed place's root local, so reborrows, fields through a reference and borrows of references keep their source borrowed; copies, moves, wraps and subtyping pass loans on; a whole-local assignment replaces what the local held and kills the loans of places through its old referent; a write through `*r` gives the value's loans to what `r` points at. A call or closure is assumed to write the loans of its other operands through each `&mut R` argument, function-typed argument, called function value and mutable capture, into whatever they point at; an operand's own loans never reach its own referent, since a slot cannot hold a borrow of itself. A local never holds a loan of a place rooted at itself.
- **Liveness ignores drops and the return place.** Only closure values among loan carriers need a drop, and dropping one reads nothing it borrowed, so a closure's borrows end at its last use. What a body returns is the callee's side of elision (`borrowck-elision`), but capture locals and `&mut R` arguments are used at every return, because the caller sees them afterwards, so a body that stores a borrow of its own local there is rejected when that local ends.
- **Codes by fix, rules by access.** `E5003` for a conflicting borrow or read, `E5004` for an assignment, `E5005` for a move and `E5006` for a value that ends while borrowed, since each has its own fix. Writes are shallow (re-pointing `r` does not touch a loan of `*r`), moves and drops deep; a call uses all its operands together, after evaluating them, so `touch(&mut u, &mut u.name)` conflicts while `x = f(r)` does not; a local the access's own location reassigns keeps nothing of its old value in scope, so `r = &mut x` may replace a `&mut x` that `r` held. Each access reports its earliest conflicting loan, and a value reported as ending while borrowed is not reported again for the same loan. Rejected: one code, and Rust's split between borrow-borrow and use-borrow conflicts, whose fixes are the same.
- **Two-phase receiver borrows come from evaluation order.** MIR takes a borrowed receiver's borrow after the arguments (`mir`), so `v.push(v.size())` needs no rule, and an argument that still holds a borrow at the call conflicts (`v.pushRef(&v.count)`). A safe call's receiver is the exception, borrowed right after its `?.` test (review, autonomous): the arguments could otherwise make the tested nullable `null` before the call (`n?.push(clear(&mut n))`). A shallow fake borrow of the tested value, which would keep reads such as `n?.push(n?.size() ?: 0)`, remains a possible relaxation.
- **Fields are disjoint by path; closures capture whole bindings.** Overlap compares projections, so different fields never conflict, a prefix always does, two indices always do, and a `Heap<T>` dereference is owned memory unlike a reference's. `{ u.name = "z" }` borrows all of `u`: precise captures would need capture paths in the typed tree and partial environments in `closures`.
- **A call's result holds every input borrow.** A call returning a reference keeps the loans of each argument whose parameter type contains a reference, in its own mode, so a `&mut` argument stays frozen: the caller's half of elision rules 1, 3 and 4, and an over-approximation of rule 2, since MIR does not yet record which operand is the receiver. A function value's result also holds its loans. A default value's generator does not keep its shared borrows of by-value earlier arguments, which the call then moves. `borrowck-elision` must make these assumptions true on the callee's side, including that a generator never returns a borrow of a by-value earlier argument. Rejected: rule 2 at call sites now; call results holding nothing until then. (`borrowck-elision` replaced this with the elision rules below.)
- **Closures capture by inferred borrow** (user). A capture is mutable when the closure writes it, borrows it mutably or contains a closure capturing it mutably, decided innermost closure first, and shared otherwise; building the closure takes those loans, which last while the closure value is live, and a closure passed straight to a call releases them when the call returns. A mutable capture of a `val` needs no `var`, as with Rust's unique borrows. The modes are recorded in elaborated MIR for `closures` (`mir`).
- **Capturing closures do not escape** (user for the rule). A function value that may hold a capture's loan or a function input escapes when it is returned (from a function, closure or default, or as a top-level `val`), stored in a field or payload, written through a reference or assigned to a captured variable (`E5007`). Writing through a reference and defaults that are capturing closures are rejected conservatively, since telling the body's storage from the caller's needs step 9; a closure without captures may escape. An escaping capture is not reported again when the captured value ends.
- **Borrowed temporaries.** `val r = &make()`, and the same borrow as a branch, `?:` fallback or block tail of an initializer or argument, live as long as the binding or the statement (`mir`). The forms not extended, `r = &make()` and a borrowed argument whose call result outlives the statement, are `E5006` with help suggesting a `val`; extension through assignments remains a possible relaxation.
- **Diagnostics follow rustc.** Titles and labels use rustc's wording: the borrow, its later use (", in later iteration of loop" when reached only through a back edge), a closure's capture with the use in its body that decided the mode, and where a borrowed value ends.

Decisions from `borrowck-elision` (the four rules agreed with the user in the borrow-checker roadmap exploration, 2026-10-17; the rest decided while proposing and implementing it, and in its review):

- **One function decides what a result is tied to, for callers and callees.** The rules are a function of a signature's kind (a function or extension, a function type, a default's generator) and its parameter types: rule 2 for an extension with a borrowed receiver (the receiver only), rules 1, 3 and 4 otherwise (every input borrow, or nothing), and, for a function type without input borrows, the function value itself. The caller's call results and the callee's return check both call it, so they agree by construction, also across files through the imported ABI, which already holds parameter types and receivers: elision needs nothing new in the ABI. Rejected: computing the two sides separately, which `borrowck-exclusivity` did and which drifted.
- **Every loan-carrying argument and capture holds an input.** An input stands for whatever the caller passed, so one item serves an escaping function input, a referent written through a `&mut R`, and the origin of a returned borrow, each role selected by the local's type. A borrow through a dereference of an argument holds that argument's input. Rejected: an item kind per role.
- **The callee's check runs at each write of the return place**, so the error points at the returned expression. Loans through a reference are judged by their root's items; other loans are memory the body owns (`E5008`) unless rooted at a closure's capture; inputs must be tied (`E5009`). Each write reports one error. Liveness still ignores the return place, so a returned local is not also `E5006`. Rejected: making the return place live at `Return` (an `E5006` that cannot say which rule applies), checking at `Return` (loses the span).
- **Borrows of top-level values hold nothing** (user for "valid everywhere"). They may be returned under any rule, and a top-level `val`'s initializer is checked as a function without inputs, so `val R: &Int = &1` is `E5008`. Rejected: promoting constant temporaries to static storage, which needs constant evaluation; it would only accept more.
- **Function types elide by rules 1, 3 and 4; only a closure whose type has no input borrow returns borrows of its captures.** An extension cannot be a function value, so rule 2 never applies to one. A call of a function value whose type has input borrows is tied to those arguments only, which lets `fun apply(g: (&Int) -> &Int, v: &Int): &Int = g(v)` pass (orchestrator), so such a closure cannot return what it captures (`E5009`); one whose type has none is tied to the function value's own borrows, its captures, so `{ &u.name }` getters work where the closure is called. A function-typed parameter without input borrows contributes only its input, which nothing ties the caller's result to, so `fun call(h: () -> &Int): &Int = h()` is `E5009`. Rejected: every closure returning its captures (unsound once `apply` passes); none doing so (rejects getters).
- **Defaults borrow only through shared references.** A generator receives `&P` for each earlier argument; it may return its slot's borrow, top-level values, and what an earlier shared reference argument or `fun` receiver refers to. Borrowing a by-value earlier argument (the caller's temporary, which the call takes, `Copy` ones included) or deriving from a `&mut` one or a `mut fun` receiver (which the call passes mutably beside it) is `E5009` at the declaration rather than an error at every call. A caller keeps exactly the slot and those arguments' borrows, not the generator's borrows of the argument temporaries. The slot still ends with the caller's statement, so `val h = hello(&u)` is `E5006`. Rejected: rejecting such defaults in the type checker, which cannot see what a borrow derives from through calls.
- **Bodies may store input borrows through inputs; by-value parameters end at each return.** Callers already assume a call writes its other operands' borrows through each `&mut R` argument and a closure into what it captures mutably, so `*slot = v` for `v: &Int` is sound. Each `Return` ends every argument that needs no drop, so storing a borrow of one there is `E5006`. Rejected: forbidding such stores; a separate code.
- **A value holds a borrow of a reference itself only when its type can refer to one** (review). A value whose type has no `&&T`, `&(&T)?` or function type points past a borrowed reference to what it refers to, whose borrows the reference's root holds, so a `&Int` read through a `&&Int`, a closure's result read through a captured reference, and a copy written through a `&mut &Int` do not keep the reference itself borrowed. Without this, `{ lead(t) }()` and `*slot = v` in a closure were false `E5008` and `E5006` errors. Rejected: tracking which reference depth each loan reaches, which the type already bounds.
- **Parameters that end are labelled at their declaration.** An `E5006` for an ending argument labels the parameter ("'v' is dropped when the function returns, while still borrowed"; closure parameters have their own spans), and a later use by the caller through a `&mut R` argument labels that parameter. Rejected: labelling the body's closing brace, which an expression body lacks.

Decisions from `generics` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **A capturing closure or function input cannot be passed to a generic slot** (`E5007`, "passed to a generic parameter here"). A parameter whose declared type has a type parameter outside a function type may be stored, returned or kept by the generic body, which is checked without knowing the instantiation. A closure passed for `calc: () -> T` is a function-typed parameter, so `remember { count }` stays allowed. This is the second half of the borrow-free rule of `generics`; owning, move-capturing closures (`closures`) will pass.

Decisions from `compound-assignment` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **One conflict per compound assignment.** Under a live mutable borrow, `x += 1` reads and writes `x` at the same span; the existing rule that, of the accesses at one span conflicting with one loan, only the last is reported gives the write's `E5004` alone, whatever the borrow's kind. `x = x + 1` keeps reporting both, since the user wrote the read and the write separately. Rejected: putting the read at the operator's span so both are reported; reporting the read's `E5003`, which would not apply to a shared borrow.

Decisions from `runtime-strings-and-drops` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **A string's character is borrowed as a temporary** (D5). `s[i]` is a value, not a place (see `type-system`), so `&s[i]`, and `s[i]` passed to a `&Char` parameter, borrow a temporary holding the character by the rules for non-places, and `val r = &s[0]; s = "cd"` is accepted, since `r` does not borrow `s`. This was a breaking change for programs relying on `&s[i]` borrowing `s`, which nothing could run.

Decisions from `payload-patterns` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Payload places overlap like the storage they share** (D16). Payloads of the same variant are disjoint, like fields, while payloads of different variants overlap, because the variants share storage. Move paths fold a payload into its enum, since nothing moves out of a payload except through a whole-value `Destructure`. A binding's borrow has its own origin (`pattern`), so `E5003`/`E5004` label it at the binding.

Decisions from `heap-boxes` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **A box reads like a reference to its contents** (D4). Every implicit read through `&T` (fields, receivers, operators, conditions, copy-out of `Copy` values, `print`, `toString()`) reads through `Heap<T>` layers too, at any depth and mixed with references, so code over a `Heap<T>` reads like code over a `&T`: a box is "an owning `&mut`". Conversions apply only where the value does not already fit, so `fun <T> same(x: &T)` given a `Heap<Int>` infers `T = Heap<Int>`. Rejected: reading only through fields and receivers (a `Heap<Int>` could not be read without moving it out); Rust's `Deref` trait coercion (no traits; these are fixed rules for one built-in type); a read form of `*h` (`*` is writes-only).
- **Owned boxes lend only to reference parameters** (D4, D21 at implementation). An owned `Heap<T>` or `Heap<T>?` lends its contents to a shared `&T` or `(&T)?` parameter, from where it lives or from a temporary, as an owned `T` is borrowed implicitly only as an argument; never to a `&mut T` parameter. A borrowed box (`&Heap<T>`, `&mut Heap<T>`, `&(Heap<T>?)`) converts wherever such a type is expected, as `&(T?)` converts to `(&T)?`, and costs nothing: the box pointer is the contents' address. `val r: &T = h` is `E4002` with help suggesting `&h`. Rejected: borrowing owned boxes implicitly in value positions, which would make boxes the one owned value borrowed without `&`.
- **`*h = v` writes a mutable box's contents** (D5), dropping the old contents and keeping the allocation. `*` peels exactly one layer, a reference or a box, so `**r = v` writes the contents of `r: &mut Heap<T>` and `*r = Heap(v)` replaces the box. A box layer must be a mutable place (a `var`, a `var` field path, or reached through `&mut`), as field writes through a box already required: a box owns its contents, as Rust's `let mut b: Box<T>` does, so unlike a `val` holding a `&mut`, a `val` box is not written through. `h = 5` for a box is `E4002` with help suggesting `*h = 5`. Rejected: writes through a `val` box; `*r = v` without the one-layer rule, ambiguous for `r: &mut Heap<Int>`.
- **The borrow checker treats a box's contents as owned by the box** (D11): `*h` and `h.x` are places under `h` with a non-reference `Deref`, so borrowing them conflicts with `h`, and moving, reassigning or ending `h` while they are borrowed is `E5005`, `E5004` or `E5006`; a borrow of a by-value box's contents cannot be returned (`E5008`), one of `r: &Heap<T>` is tied to `r`. Boxing a capturing closure is an escape (`E5007`, "stored in a box here"), like storing it in a struct field, because a box can be returned or stored anywhere; `closures` and `escaping-closures` revisit both together.
- **Safe navigation is still a temporary** (from the review). `f(t.right?.left)` for a `(&Tree)?` parameter moves the field out (`E4017`) as for any `T?`: lending a `?.` chain of fields read in place is a possible relaxation of "A borrow points at one place or one temporary", recorded in `ROADMAP.md`.

Decisions from `string-templates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **Template parts are borrowed until the template is built, scalars read at their position** (D11). Parts are evaluated left to right. A `String` part is a shared `Operator` borrow, as an operand of `+` is, used where the template is built, so a later part mutating it conflicts (`E5003`), and a template never moves a part (`"$s!"` leaves `s` usable). A scalar part is copied to a temporary first when a later part may write it (`"$n ${bump(&mut n)} $n"` gives `1 2 2`, as in Kotlin). A template of many string parts keeps that many shared loans live at once, the `live-borrows` shape of the compile-time benchmark. Rejected: copying string parts in Kotlin's style (a hidden copy per part), and moving temporary parts into the runtime call to reuse their buffers, a later optimization.
