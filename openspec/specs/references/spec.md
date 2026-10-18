# References Specification

## Purpose
The references specification defines how borrowed references are used: as the type checker enforces it, where borrows are created implicitly, how values are read and written through references, how reference bindings are re-pointed and reborrowed, how borrowed nullables are viewed, and how reference types relate; and as the borrow checker enforces it, how long each borrow lasts, which accesses conflict with it, what a call's result keeps borrowed, how closures borrow what they capture, and why a capturing closure cannot escape. What a function may return a borrow of (the callee's side of the elision rules) is added here by `borrowck-elision`.

## Requirements

### Requirement: Implicit Shared Borrow at Argument Sites
Where a call's parameter has type `&T`, an argument of type `T` MUST be borrowed implicitly. A place argument (a variable, a `.` field path, or an index) MUST be borrowed where it lives, without being consumed. Any other argument MUST be evaluated as a temporary, in a consuming position, that lives until the end of the statement. Default values are arguments. Elsewhere a borrow MUST be written with `&`.

#### Scenario: Implicit shared borrow at argument sites
- **WHEN** `fun greet(name: &String)` and `struct User { val name: String }` are declared and a body of `fun f(u: &User, name: String)` runs `greet(name)`, `greet(u.name)` and `greet("Ada")`
- **THEN** the body typechecks successfully, and neither `name` nor `u.name` is reported as moved

#### Scenario: Borrowing the result of a call
- **WHEN** `fun make(): String` and `fun greet(name: &String)` are declared and `fun f() { greet(make()) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: A branch result is a temporary
- **WHEN** `fun greet(name: &String)` and `struct User { val name: String }` are declared, and `fun f(c: Bool, u: User, v: User) { greet(if (c) u.name else v.name) }` and `fun g(c: Bool) { greet(if (c) "a" else "b") }` are checked
- **THEN** the type checker emits `E4017` for `u.name` and for `v.name`, and accepts `g`

#### Scenario: A default value is borrowed like an argument
- **WHEN** `fun greet(name: &String = "guest")` is declared and `fun f() { greet() }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Calling a function-typed value
- **WHEN** `fun f(g: (&String) -> Unit, s: String) { g(s) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Borrows outside arguments are written
- **WHEN** `struct User { val name: String }` is declared, and `fun f(name: String) { val r: &String = name }` and `fun g(u: &User): &String = u.name` are checked
- **THEN** the type checker emits `E4002` for `f`, with help suggesting `&name`, and `E4005` for `g`, with help suggesting `&u.name`

### Requirement: Explicit Mutable Borrows at Call Sites
A parameter of type `&mut T` MUST NOT borrow an owned argument implicitly. The caller MUST write `&mut`, and an owned place passed without it MUST be rejected with `E4004`, with help suggesting `&mut` before the argument. The receiver of a `mut fun` call is the one exception: it is borrowed mutably without `&mut` (see "Mutable Receivers Must Be Mutable Places").

#### Scenario: Explicit mutation at call sites
- **WHEN** `fun bump(r: &mut Int)` is declared, and a body declares `var x = 1` and calls `bump(x)` and `bump(&mut x)`
- **THEN** the type checker emits `E4004` for `bump(x)`, with help suggesting `&mut x`, and accepts `bump(&mut x)`

### Requirement: Passing a Mutable Borrow Reborrows It
A place of type `&mut T` passed as an argument where `&mut T` or `&T` is expected MUST be reborrowed for the call rather than moved, so that it stays usable afterwards. `&mut` applied to a `val` binding MUST be rejected with `E4003`, whatever the binding's type.

#### Scenario: Passing a mutable borrow twice
- **WHEN** `fun bump(r: &mut Int)` and `fun read(r: &Int): Int` are declared and `fun f(r: &mut Int): Int { bump(r); bump(r); return read(r) }` is checked
- **THEN** the file typechecks successfully, and each call reborrows `r` without `&mut`

#### Scenario: Mutable borrow of a val reference binding rejected
- **WHEN** `fun f(r: &mut Int) { val m = &mut r }` is checked
- **THEN** the type checker emits `E4003`, because `&mut r` borrows the `val` binding `r` itself

### Requirement: Copy-Out From References
Where a value of type `T` is expected and the expression has a reference type (`&T`, `&mut T` or nested borrows of `T`), a `Copy` value MUST be copied out implicitly, at every value position: initializers with a type, assigned values, returned values, by-value arguments, constructor fields and payloads, and default values. `(&T)?` MUST be accepted where `T?` is expected when `T` is `Copy`. An unannotated initializer MUST keep the reference type.

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

### Requirement: Writing Through a Mutable Reference
`*r = v` MUST write `v` into the place `r` refers to. Every reference layer written through MUST be `&mut`, or the assignment is rejected with `E4003`. A `*` applied to a value that is not a reference MUST be rejected with `NotAReference` (`E4019`). The value MUST be checked against the referent type in a consuming position, and the binding holding the reference MAY be a `val`.

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
- **THEN** the type checker emits `E4019` naming the type `Int`

#### Scenario: The written value is checked and consumed
- **WHEN** `struct User { val name: String }` is declared and a body of `fun f(r: &mut String, a: &Int, n: &mut Int, s: String, u: User)` runs `*n = a`, `*r = s`, `*r = u.name` and `*n = "x"`
- **THEN** `*n = a` (a copy-out) and `*r = s` (a move of a whole parameter) are accepted, the type checker emits `E4017` for `u.name`, and it emits `E4002` for `"x"`

#### Scenario: Writing through the result of a call
- **WHEN** `fun pick(a: &mut Int, b: &mut Int): &mut Int` is declared and a body declares `var a = 1` and `var b = 2` and runs `*pick(&mut a, &mut b) = 0`
- **THEN** the body typechecks successfully

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
The result of a call whose type contains a reference MUST hold every borrow passed to the call's input borrows (its parameters whose type contains a reference, other than function types), each in its own mode, so a `&mut` argument MUST stay frozen while the result is live. The result of calling a function value MUST also hold the borrows of the function value. The borrows a default value's generator takes of earlier by-value arguments MUST NOT be held by its result.

#### Scenario: A returned borrow keeps every input borrowed
- **WHEN** `fun first(a: &String, b: &String): &String = a`, `fun take(s: String) {}` and `fun greet(s: &String): Int = 1` are declared, and `fun f(x: String, y: String) { val r = first(&x, &y); take(y); greet(r) }` and `fun g(x: String, y: String) { val r = first(&x, &y); greet(r); take(y) }` are checked
- **THEN** the borrow checker emits `E5005` at `y` in `take(y)` of `f`, and accepts `g`

#### Scenario: A mutable input stays frozen while the result is live
- **WHEN** `fun pick(a: &mut Int): &Int = a` and `fun print(n: &Int) {}` are declared and `fun f() { var x = 1; val r = pick(&mut x); print(x); print(r) }` is checked
- **THEN** the borrow checker emits `E5003` at `x` in `print(x)`

#### Scenario: A method result keeps its receiver borrowed
- **WHEN** `struct Team { var lead: String }`, `fun Team.captain(): &String = &lead` and `fun greet(s: &String): Int = 1` are declared and a body runs `var t = Team("a"); val c = t.captain(); t.lead = "b"; greet(c)`
- **THEN** the borrow checker emits `E5004` at `t.lead`

#### Scenario: An omitted reference default after a by-value parameter
- **WHEN** `fun greet(a: String, n: &String = "anon"): Int = 1` is declared and `fun f(s: String): Int = greet(s)` is checked
- **THEN** the file passes `prism check`

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
A function value that may be a closure with captures, or a function-typed parameter, MUST NOT escape: being returned from a function, closure or default value, initializing a top-level `val`, being stored in a struct field or enum payload, being written through a reference, or being assigned to a captured variable MUST be rejected by the borrow checker with `EscapingClosure` (`E5007`). A closure without captures MAY escape, and passing any closure to a call is not an escape.

#### Scenario: Capturing closures do not escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): () -> Int = { greet(s) }`, `fun g(s: String): Button = Button({ greet(s) })`, `fun h(slot: &mut (() -> Int), s: String) { *slot = { greet(s) } }`, `fun k(p: () -> Int): () -> Int = p` and `fun m(a: Int, p: () -> Int = { a }): Int = p()` are checked
- **THEN** the borrow checker emits `E5007` for each: for `f` saying that a closure that captures `s` cannot escape and labelling where it is returned, for `g`, `h` and `m` likewise, and for `k` saying that the function-typed parameter `p` cannot escape

#### Scenario: A closure without captures may escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun run(f: () -> Int): Int = f()` are declared, and `fun f(): () -> Int = { 1 }`, `fun g(): Button = Button({ 2 })` and `fun h(s: String): Int = run { s.clone(); 3 }` are checked
- **THEN** the file passes `prism check`

### Requirement: Borrow Conflict Diagnostics Show the Borrow and Its Later Use
A diagnostic of `E5003`, `E5004` or `E5005` MUST point at the conflicting access and name the place, and MUST list as related locations where the conflicting borrow was created and where it is used later, with ", in later iteration of loop" when that use is reached only through a loop's back edge. A borrow made by a closure MUST also label the use inside the closure that decided how it is captured.

#### Scenario: Conflict points at the borrow and its later use
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f() { var x = 1; val r = &x; x = 2; print(r) }`
- **THEN** it reports `error[E5004]: cannot assign to 'x' because it is borrowed` at `x` in `x = 2`, labelled "'x' is assigned here while borrowed", with a related location at `&x` labelled "'x' is borrowed here" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Closure borrow labels
- **WHEN** `struct User { var name: String }`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `prism check` runs on `fun f() { var u = User("a"); val g = { u.name = "z"; 1 }; greet(u.name); run(g) }`
- **THEN** it reports `error[E5003]: cannot borrow 'u.name' as shared because 'u' is borrowed as mutable` at `u.name` in `greet(u.name)`, labelled "shared borrow occurs here", with related locations at the closure labelled "closure borrows 'u' mutably here", at `u.name` inside the closure labelled "borrow occurs due to this use of 'u' in the closure", and at `g` in `run(g)` labelled "mutable borrow later used here"

### Requirement: Lifetime and Escape Diagnostics Show Where the Value Ends
An `E5006` diagnostic MUST point at the borrow, with related locations where the borrowed value ends and where the borrow is used later. An `E5007` diagnostic MUST point at where the value escapes, with a related location at the capture or the parameter.

#### Scenario: Dropped value labels
- **WHEN** `fun print(n: &Int) {}` is declared and `prism check` runs on `fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }`
- **THEN** it reports `error[E5006]: 'n' does not live long enough` at `&n` labelled "borrowed value does not live long enough", with a related location at the `}` closing the branch labelled "'n' dropped here while still borrowed" and one at `r` in `print(r)` labelled "borrow later used here"

#### Scenario: Escaping closure labels
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `prism check` runs on `fun f(s: String): () -> Int = { greet(s) }`
- **THEN** it reports `error[E5007]: a closure that captures 's' cannot escape` at the closure labelled "returned here", with a related location at `s` inside the closure labelled "'s' is captured here", a note that a closure borrows what it captures, and help suggesting to pass `s` to the closure as a parameter

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
- **A call's result holds every input borrow.** A call returning a reference keeps the loans of each argument whose parameter type contains a reference, in its own mode, so a `&mut` argument stays frozen: the caller's half of elision rules 1, 3 and 4, and an over-approximation of rule 2, since MIR does not yet record which operand is the receiver. A function value's result also holds its loans. A default value's generator does not keep its shared borrows of by-value earlier arguments, which the call then moves. `borrowck-elision` must make these assumptions true on the callee's side, including that a generator never returns a borrow of a by-value earlier argument. Rejected: rule 2 at call sites now; call results holding nothing until then.
- **Closures capture by inferred borrow** (user). A capture is mutable when the closure writes it, borrows it mutably or contains a closure capturing it mutably, decided innermost closure first, and shared otherwise; building the closure takes those loans, which last while the closure value is live, and a closure passed straight to a call releases them when the call returns. A mutable capture of a `val` needs no `var`, as with Rust's unique borrows. The modes are recorded in elaborated MIR for `closures` (`mir`).
- **Capturing closures do not escape** (user for the rule). A function value that may hold a capture's loan or a function input escapes when it is returned (from a function, closure or default, or as a top-level `val`), stored in a field or payload, written through a reference or assigned to a captured variable (`E5007`). Writing through a reference and defaults that are capturing closures are rejected conservatively, since telling the body's storage from the caller's needs step 9; a closure without captures may escape. An escaping capture is not reported again when the captured value ends.
- **Borrowed temporaries.** `val r = &make()`, and the same borrow as a branch, `?:` fallback or block tail of an initializer or argument, live as long as the binding or the statement (`mir`). The forms not extended, `r = &make()` and a borrowed argument whose call result outlives the statement, are `E5006` with help suggesting a `val`; extension through assignments remains a possible relaxation.
- **Diagnostics follow rustc.** Titles and labels use rustc's wording: the borrow, its later use (", in later iteration of loop" when reached only through a back edge), a closure's capture with the use in its body that decided the mode, and where a borrowed value ends.
